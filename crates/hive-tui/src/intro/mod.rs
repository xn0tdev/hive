//! First-run intro wizard — About-style centered panel, keyboard only.

mod draw;
mod presets;

use std::io::{self, IsTerminal};
use std::time::Duration;

use comb::{Event, Key, KeyCode, Terminal};
use hive_core::config::SearchBackend;
use hive_llm::catalog::{
    enrich_models, fetch_models_dev, list_provider_models, suggest_roles, ModelCard, RolePicks,
};
use tokio::sync::oneshot;

pub use presets::{ProviderPreset, PRESETS};

/// Prefill from an existing `AppConfig` so Provider/Models can offer “use existing”.
#[derive(Debug, Clone, Default)]
pub struct IntroPrefill {
    pub provider_base_url: String,
    pub provider_api_key_env: String,
    pub provider_api_key: String,
    pub default_id: String,
    pub default_name: String,
    pub search_backend: SearchBackend,
    pub search_api_key: Option<String>,
}

/// Options for [`run_intro`].
#[derive(Debug, Clone)]
pub struct IntroOpts {
    pub version: String,
    /// Values from the current config (if any).
    pub prefill: Option<IntroPrefill>,
}

/// Result of the intro wizard.
#[derive(Debug, Clone)]
pub enum IntroResult {
    /// Wizard finished with choices ready to write.
    Completed(Box<SetupDraft>),
    /// User aborted (Esc on welcome, or quit).
    Abort,
}

/// Everything needed to write `config.toml` after a completed intro.
#[derive(Debug, Clone)]
pub struct SetupDraft {
    pub provider_base_url: String,
    pub provider_api_key_env: String,
    pub provider_api_key: String,
    pub default_id: String,
    pub default_name: String,
    pub search_backend: SearchBackend,
    pub search_api_key: Option<String>,
    pub setup_complete: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Welcome,
    Modes,
    /// Choose provider (list + description only).
    Provider,
    /// Enter API key (and custom URL/env when needed).
    ProviderKey,
    Models,
    Search,
    Done,
}

/// Focus within the ProviderKey step (list lives on its own step).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderKeyFocus {
    Key,
    CustomUrl,
    CustomEnv,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchFocus {
    List,
    Key,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchChoice {
    Exa,
    Perplexity,
    Skip,
}

#[derive(Debug, Clone)]
struct RoleAssignment {
    id: String,
    name: String,
}

impl RoleAssignment {
    fn empty() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
        }
    }

    fn from_card(c: &ModelCard) -> Self {
        Self {
            id: c.id.clone(),
            name: c.name.clone(),
        }
    }

    fn from_parts(id: &str, name: &str) -> Self {
        Self {
            id: id.to_string(),
            name: if name.is_empty() {
                id.rsplit('/').next().unwrap_or(id).to_string()
            } else {
                name.to_string()
            },
        }
    }
}

enum FetchState {
    Idle,
    Loading,
    Ready(Vec<ModelCard>),
    Err(String),
}

struct IntroState {
    opts: IntroOpts,
    step: Step,
    theme: crate::theme::Theme,
    /// Provider list cursor.
    provider_idx: usize,
    provider_key_focus: ProviderKeyFocus,
    provider_key: String,
    custom_url: String,
    custom_env: String,
    /// Model list.
    fetch: FetchState,
    fetch_rx: Option<oneshot::Receiver<Result<Vec<ModelCard>, String>>>,
    model_idx: usize,
    model_scroll: usize,
    /// Incremental type-to-filter over id/name (Models step, Ready only).
    model_filter: String,
    /// Catalog heuristic pick — used to mark recommended rows.
    suggested: RolePicks,
    model: RoleAssignment,
    /// Search.
    search_idx: usize,
    search_focus: SearchFocus,
    search_key: String,
    status: String,
    /// Idle animation tick (blink / welcome accent).
    tick: u64,
}

impl IntroState {
    fn new(opts: IntroOpts) -> Self {
        let mut state = IntroState {
            opts,
            step: Step::Welcome,
            theme: crate::theme::Theme::gray(),
            provider_idx: 0,
            provider_key_focus: ProviderKeyFocus::Key,
            provider_key: String::new(),
            custom_url: String::new(),
            custom_env: "LLM_API_KEY".into(),
            fetch: FetchState::Idle,
            fetch_rx: None,
            model_idx: 0,
            model_scroll: 0,
            model_filter: String::new(),
            suggested: RolePicks::default(),
            model: RoleAssignment::empty(),
            search_idx: 0,
            search_focus: SearchFocus::List,
            search_key: String::new(),
            status: String::new(),
            tick: 0,
        };
        state.apply_prefill();
        state
    }

    fn apply_prefill(&mut self) {
        let Some(p) = self.opts.prefill.clone() else {
            return;
        };

        // Match preset by base URL; fall back to Custom.
        if let Some(i) = PRESETS
            .iter()
            .position(|pr| !pr.is_custom && pr.base_url == p.provider_base_url)
        {
            self.provider_idx = i;
        } else if !p.provider_base_url.is_empty() {
            self.provider_idx = PRESETS
                .iter()
                .position(|pr| pr.is_custom)
                .unwrap_or(PRESETS.len() - 1);
            self.custom_url = p.provider_base_url.clone();
            if !p.provider_api_key_env.is_empty() {
                self.custom_env = p.provider_api_key_env.clone();
            }
        }

        if !p.provider_api_key.is_empty() {
            self.provider_key = p.provider_api_key;
        }

        if !p.default_id.is_empty() {
            self.model = RoleAssignment::from_parts(&p.default_id, &p.default_name);
        }

        self.search_idx = match p.search_backend {
            SearchBackend::Exa => 0,
            SearchBackend::Perplexity => 1,
            SearchBackend::None => 2,
        };
        if let Some(k) = p.search_api_key {
            self.search_key = k;
        }
    }

    /// Prefill (or typed key) ready — offer “use existing” instead of re-entering.
    fn can_use_existing_provider(&self) -> bool {
        !self.provider_key.trim().is_empty()
            && (!self.preset().is_custom || !self.custom_url.trim().is_empty())
    }

    /// Model already filled — keep pick while catalog loads (not when list is Ready:
    /// then `u`/`s` are filter keys).
    fn can_use_existing_models(&self) -> bool {
        self.models_ready() && !matches!(self.fetch, FetchState::Ready(_))
    }

    fn can_use_existing_search(&self) -> bool {
        self.opts.prefill.is_some()
    }

    fn preset(&self) -> &'static ProviderPreset {
        &PRESETS[self.provider_idx.min(PRESETS.len() - 1)]
    }

    fn resolved_base_url(&self) -> String {
        let p = self.preset();
        if p.is_custom {
            self.custom_url.trim().to_string()
        } else {
            p.base_url.to_string()
        }
    }

    fn resolved_api_key_env(&self) -> String {
        let p = self.preset();
        if p.is_custom {
            let e = self.custom_env.trim();
            if e.is_empty() {
                "LLM_API_KEY".into()
            } else {
                e.to_string()
            }
        } else {
            p.api_key_env.to_string()
        }
    }

    fn search_choice(&self) -> SearchChoice {
        match self.search_idx {
            0 => SearchChoice::Exa,
            1 => SearchChoice::Perplexity,
            _ => SearchChoice::Skip,
        }
    }

    fn models_ready(&self) -> bool {
        !self.model.id.is_empty()
    }

    fn to_draft(&self) -> Option<SetupDraft> {
        if self.provider_key.trim().is_empty() || !self.models_ready() {
            return None;
        }

        let (search_backend, search_api_key) = match self.search_choice() {
            SearchChoice::Exa => (
                SearchBackend::Exa,
                Some(self.search_key.trim().to_string()).filter(|s| !s.is_empty()),
            ),
            SearchChoice::Perplexity => (
                SearchBackend::Perplexity,
                Some(self.search_key.trim().to_string()).filter(|s| !s.is_empty()),
            ),
            SearchChoice::Skip => (SearchBackend::None, None),
        };

        Some(SetupDraft {
            provider_base_url: self.resolved_base_url(),
            provider_api_key_env: self.resolved_api_key_env(),
            provider_api_key: self.provider_key.trim().to_string(),
            default_id: self.model.id.clone(),
            default_name: self.model.name.clone(),
            search_backend,
            search_api_key,
            setup_complete: true,
        })
    }

    fn start_model_fetch(&mut self) {
        let base = self.resolved_base_url();
        let key = self.provider_key.trim().to_string();
        let hint = self.preset().models_dev_hint.map(|s| s.to_string());
        if base.is_empty() || key.is_empty() {
            self.fetch = FetchState::Err("need base URL and API key".into());
            return;
        }
        self.fetch = FetchState::Loading;
        self.status = "fetching models…".into();
        let (tx, rx) = oneshot::channel();
        self.fetch_rx = Some(rx);
        tokio::spawn(async move {
            let listed = match list_provider_models(&base, &key).await {
                Ok(m) => m,
                Err(e) => {
                    let _ = tx.send(Err(e.to_string()));
                    return;
                }
            };
            let catalog = fetch_models_dev().await.ok();
            let cards = enrich_models(&listed, catalog.as_ref(), hint.as_deref());
            let _ = tx.send(Ok(cards));
        });
    }

    fn poll_fetch(&mut self) {
        let Some(rx) = self.fetch_rx.as_mut() else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(cards)) => {
                self.fetch_rx = None;
                self.apply_cards(cards);
            }
            Ok(Err(e)) => {
                self.fetch_rx = None;
                self.fetch = FetchState::Err(e);
                self.status = "could not list models — go back and check key/URL".into();
            }
            Err(oneshot::error::TryRecvError::Empty) => {}
            Err(oneshot::error::TryRecvError::Closed) => {
                self.fetch_rx = None;
                self.fetch = FetchState::Err("fetch cancelled".into());
            }
        }
    }

    fn apply_cards(&mut self, cards: Vec<ModelCard>) {
        if cards.is_empty() {
            self.fetch = FetchState::Err("provider returned no models".into());
            self.status.clear();
            return;
        }
        let picks = suggest_roles(&cards);
        let find = |id: Option<&String>| {
            id.and_then(|id| cards.iter().find(|c| &c.id == id))
                .map(RoleAssignment::from_card)
        };
        // Keep prefilled / prior pick; only suggest when still empty.
        if self.model.id.is_empty() {
            self.model = find(picks.default.as_ref())
                .or_else(|| cards.first().map(RoleAssignment::from_card))
                .unwrap_or_else(RoleAssignment::empty);
        } else if let Some(c) = cards.iter().find(|c| c.id == self.model.id) {
            self.model = RoleAssignment::from_card(c);
        }
        self.suggested = picks;
        self.model_filter.clear();
        self.model_scroll = 0;
        self.status.clear();
        self.fetch = FetchState::Ready(cards);
        self.sync_model_cursor();
    }

    fn suggested_id(&self) -> Option<&str> {
        self.suggested.default.as_deref()
    }

    /// Jump list cursor to the selected model (or first visible).
    fn sync_model_cursor(&mut self) {
        let want = self.model.id.clone();
        let indices = self.visible_model_indices();
        if indices.is_empty() {
            return;
        }
        let FetchState::Ready(cards) = &self.fetch else {
            return;
        };
        if let Some(pos) = indices.iter().position(|&i| cards[i].id == want) {
            self.model_idx = indices[pos];
        } else {
            self.model_idx = indices[0];
        }
        self.model_scroll = 0;
    }

    fn clamp_model_cursor(&mut self) {
        let indices = self.visible_model_indices();
        if indices.is_empty() {
            return;
        }
        if !indices.contains(&self.model_idx) {
            self.model_idx = indices[0];
        }
    }

    pub(super) fn visible_model_indices(&self) -> Vec<usize> {
        let FetchState::Ready(cards) = &self.fetch else {
            return Vec::new();
        };
        let filter = self.model_filter.to_ascii_lowercase();
        cards
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                if filter.is_empty() {
                    return true;
                }
                c.id.to_ascii_lowercase().contains(&filter)
                    || c.name.to_ascii_lowercase().contains(&filter)
            })
            .map(|(i, _)| i)
            .collect()
    }
}

pub(super) fn short_model_label(id: &str, name: &str, max: usize) -> String {
    let raw = if !name.is_empty() {
        name
    } else {
        id.rsplit('/').next().unwrap_or(id)
    };
    let n = raw.chars().count();
    if n <= max {
        return raw.to_string();
    }
    if max <= 1 {
        return "…".into();
    }
    let mut out: String = raw.chars().take(max - 1).collect();
    out.push('…');
    out
}

/// Run the first-run intro. Requires an interactive terminal and a Tokio runtime.
pub async fn run_intro(opts: IntroOpts) -> io::Result<IntroResult> {
    if !io::stdout().is_terminal() {
        return Err(io::Error::other(
            "hive setup must be run in an interactive terminal",
        ));
    }
    let mut terminal = Terminal::new()?;
    let _ = terminal.mouse_mode(comb::MouseMode::Off);
    let mut state = IntroState::new(opts);
    let mut dirty = true;

    let result = loop {
        state.poll_fetch();

        if dirty || matches!(state.fetch, FetchState::Loading) {
            let mut cursor = None;
            terminal.draw(|f| {
                cursor = draw::paint(f, &state);
                if let Some((x, y)) = cursor {
                    f.set_cursor(x, y);
                }
            })?;
            dirty = false;
        }

        let wait = if matches!(state.fetch, FetchState::Loading) {
            Duration::from_millis(50)
        } else {
            Duration::from_millis(200)
        };

        if let Some(ev) = terminal.read_event(wait)? {
            match ev {
                Event::Key(key) => {
                    if let Some(res) = handle_key(&mut state, key) {
                        break res;
                    }
                    dirty = true;
                }
                Event::Paste(text) => {
                    handle_paste(&mut state, &text);
                    dirty = true;
                }
                Event::Resize(_, _) => dirty = true,
                Event::Mouse(_) => {}
            }
        } else {
            // Idle tick for subtle blink / welcome accent.
            state.tick = state.tick.wrapping_add(1);
            dirty = true;
        }
    };

    // Terminal Drop restores the screen.
    drop(terminal);
    Ok(result)
}

fn sanitize_api_key(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

fn handle_paste(state: &mut IntroState, text: &str) {
    let cleaned = sanitize_api_key(text);
    if cleaned.is_empty() {
        return;
    }
    match state.step {
        Step::Provider => {
            // Paste on the list jumps into the key step for the selected provider.
            enter_provider_key_step(state);
            state.provider_key.push_str(&cleaned);
            state.status.clear();
        }
        Step::ProviderKey => {
            state.provider_key_focus = ProviderKeyFocus::Key;
            state.provider_key.push_str(&cleaned);
            state.status.clear();
        }
        Step::Search if state.search_choice() != SearchChoice::Skip => {
            state.search_focus = SearchFocus::Key;
            state.search_key.push_str(&cleaned);
        }
        _ => {}
    }
}

fn read_clipboard_text() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok()
}

fn paste_from_clipboard(state: &mut IntroState) {
    if let Some(text) = read_clipboard_text() {
        handle_paste(state, &text);
    }
}

fn handle_key(state: &mut IntroState, key: Key) -> Option<IntroResult> {
    if key.mods.ctrl && matches!(key.code, KeyCode::Char('c')) {
        return Some(IntroResult::Abort);
    }

    match state.step {
        Step::Welcome => handle_welcome(state, key),
        Step::Modes => handle_modes(state, key),
        Step::Provider => handle_provider(state, key),
        Step::ProviderKey => handle_provider_key(state, key),
        Step::Models => handle_models(state, key),
        Step::Search => handle_search(state, key),
        Step::Done => handle_done(state, key),
    }
}

fn handle_welcome(state: &mut IntroState, key: Key) -> Option<IntroResult> {
    match key.code {
        KeyCode::Esc => Some(IntroResult::Abort),
        KeyCode::Enter | KeyCode::Right => {
            state.step = Step::Modes;
            None
        }
        _ => None,
    }
}

fn handle_modes(state: &mut IntroState, key: Key) -> Option<IntroResult> {
    match key.code {
        KeyCode::Left => {
            state.step = Step::Welcome;
            None
        }
        KeyCode::Enter | KeyCode::Right => {
            state.step = Step::Provider;
            None
        }
        KeyCode::Esc => None,
        _ => None,
    }
}

/// Move to Models and always load the catalog so the list is never barren.
fn advance_from_provider(state: &mut IntroState) {
    state.status.clear();
    state.step = Step::Models;
    state.start_model_fetch();
}

fn enter_provider_key_step(state: &mut IntroState) {
    state.status.clear();
    state.step = Step::ProviderKey;
    state.provider_key_focus = if state.preset().is_custom {
        ProviderKeyFocus::CustomUrl
    } else {
        ProviderKeyFocus::Key
    };
}

/// Step A — choose provider (names + description only).
fn handle_provider(state: &mut IntroState, key: Key) -> Option<IntroResult> {
    // Use existing keeps prefilled / already typed key without re-entering.
    if !key.mods.ctrl
        && !key.mods.alt
        && matches!(
            key.code,
            KeyCode::Char('u') | KeyCode::Char('U') | KeyCode::Char('s') | KeyCode::Char('S')
        )
        && state.can_use_existing_provider()
    {
        advance_from_provider(state);
        return None;
    }

    // Ctrl+V / Insert → clipboard into the key step.
    if (key.mods.ctrl && matches!(key.code, KeyCode::Char('v') | KeyCode::Char('V')))
        || matches!(key.code, KeyCode::Insert)
    {
        enter_provider_key_step(state);
        paste_from_clipboard(state);
        return None;
    }

    match key.code {
        KeyCode::Esc | KeyCode::Left => {
            state.step = Step::Modes;
            None
        }
        KeyCode::Up => {
            if state.provider_idx > 0 {
                state.provider_idx -= 1;
            }
            None
        }
        KeyCode::Down => {
            if state.provider_idx + 1 < PRESETS.len() {
                state.provider_idx += 1;
            }
            None
        }
        KeyCode::Enter | KeyCode::Right => {
            enter_provider_key_step(state);
            None
        }
        _ => None,
    }
}

/// Step B — API key (custom URL / env when needed).
fn handle_provider_key(state: &mut IntroState, key: Key) -> Option<IntroResult> {
    let custom = state.preset().is_custom;

    if (key.mods.ctrl && matches!(key.code, KeyCode::Char('v') | KeyCode::Char('V')))
        || matches!(key.code, KeyCode::Insert)
    {
        state.provider_key_focus = ProviderKeyFocus::Key;
        paste_from_clipboard(state);
        return None;
    }

    match key.code {
        KeyCode::Esc | KeyCode::Left => {
            // Keep typed key / custom fields when returning to the list.
            state.status.clear();
            state.step = Step::Provider;
            None
        }
        KeyCode::Tab => {
            state.provider_key_focus = cycle_provider_key_focus(state.provider_key_focus, custom);
            None
        }
        KeyCode::Enter | KeyCode::Right => match state.provider_key_focus {
            ProviderKeyFocus::CustomUrl => {
                if state.custom_url.trim().is_empty() {
                    state.status = "enter a base URL".into();
                    None
                } else {
                    state.provider_key_focus = ProviderKeyFocus::CustomEnv;
                    state.status.clear();
                    None
                }
            }
            ProviderKeyFocus::CustomEnv => {
                state.provider_key_focus = ProviderKeyFocus::Key;
                state.status.clear();
                None
            }
            ProviderKeyFocus::Key => {
                if state.provider_key.trim().is_empty() {
                    state.status = "paste an API key".into();
                    return None;
                }
                if custom && state.custom_url.trim().is_empty() {
                    state.status = "enter a base URL".into();
                    state.provider_key_focus = ProviderKeyFocus::CustomUrl;
                    return None;
                }
                state.status.clear();
                state.step = Step::Models;
                state.start_model_fetch();
                None
            }
        },
        KeyCode::Backspace => {
            match state.provider_key_focus {
                ProviderKeyFocus::Key => {
                    state.provider_key.pop();
                }
                ProviderKeyFocus::CustomUrl => {
                    state.custom_url.pop();
                }
                ProviderKeyFocus::CustomEnv => {
                    state.custom_env.pop();
                }
            }
            None
        }
        KeyCode::Char(c) if !key.mods.ctrl && !key.mods.alt => {
            match state.provider_key_focus {
                ProviderKeyFocus::Key => state.provider_key.push(c),
                ProviderKeyFocus::CustomUrl => state.custom_url.push(c),
                ProviderKeyFocus::CustomEnv => state.custom_env.push(c),
            }
            None
        }
        _ => None,
    }
}

fn cycle_provider_key_focus(f: ProviderKeyFocus, custom: bool) -> ProviderKeyFocus {
    if custom {
        match f {
            ProviderKeyFocus::CustomUrl => ProviderKeyFocus::CustomEnv,
            ProviderKeyFocus::CustomEnv => ProviderKeyFocus::Key,
            ProviderKeyFocus::Key => ProviderKeyFocus::CustomUrl,
        }
    } else {
        ProviderKeyFocus::Key
    }
}

fn handle_models(state: &mut IntroState, key: Key) -> Option<IntroResult> {
    // While catalog loads: keep existing role picks and continue.
    if !key.mods.ctrl
        && !key.mods.alt
        && matches!(
            key.code,
            KeyCode::Char('u') | KeyCode::Char('U') | KeyCode::Char('s') | KeyCode::Char('S')
        )
        && state.model_filter.is_empty()
        && state.can_use_existing_models()
    {
        state.status.clear();
        state.step = Step::Search;
        return None;
    }

    // Idle (e.g. after back): first ↓/Enter kicks off fetch so the list appears.
    if matches!(state.fetch, FetchState::Idle)
        && matches!(
            key.code,
            KeyCode::Enter | KeyCode::Down | KeyCode::Up | KeyCode::Right
        )
    {
        state.start_model_fetch();
        return None;
    }

    match key.code {
        KeyCode::Esc => {
            if !state.model_filter.is_empty() {
                state.model_filter.clear();
                state.clamp_model_cursor();
                state.status.clear();
                return None;
            }
            models_go_back(state);
            None
        }
        KeyCode::Right => {
            if state.models_ready() {
                state.model_filter.clear();
                state.status.clear();
                state.step = Step::Search;
            }
            None
        }
        KeyCode::Enter => assign_model(state),
        KeyCode::Up => {
            move_model(state, -1);
            None
        }
        KeyCode::Down => {
            move_model(state, 1);
            None
        }
        KeyCode::Backspace => {
            if !state.model_filter.is_empty() {
                state.model_filter.pop();
                state.clamp_model_cursor();
                state.status.clear();
            }
            None
        }
        KeyCode::Char(c) if !key.mods.ctrl && !key.mods.alt && model_filter_char(c) => {
            if matches!(state.fetch, FetchState::Ready(_)) {
                state.model_filter.push(c);
                state.clamp_model_cursor();
                state.status.clear();
            }
            None
        }
        _ => None,
    }
}

fn models_go_back(state: &mut IntroState) {
    state.fetch_rx = None;
    state.fetch = FetchState::Idle;
    state.model_filter.clear();
    state.step = Step::ProviderKey;
    state.provider_key_focus = ProviderKeyFocus::Key;
}

fn model_filter_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | ':' | '+')
}

/// Enter: assign focused model to the single slot.
fn assign_model(state: &mut IntroState) -> Option<IntroResult> {
    if matches!(state.fetch, FetchState::Loading | FetchState::Idle) {
        return None;
    }
    if !matches!(state.fetch, FetchState::Ready(_)) {
        return None;
    }
    let indices = state.visible_model_indices();
    if indices.is_empty() {
        state.status = "no models match filter".into();
        return None;
    }
    let pos = indices
        .iter()
        .position(|&i| i == state.model_idx)
        .unwrap_or(0);
    let card_i = indices[pos];
    let card = match &state.fetch {
        FetchState::Ready(cards) => cards[card_i].clone(),
        _ => return None,
    };
    state.model = RoleAssignment::from_card(&card);
    state.model_filter.clear();
    state.status = "model set · → continue".into();
    None
}

fn move_model(state: &mut IntroState, delta: isize) {
    let indices = state.visible_model_indices();
    if indices.is_empty() {
        return;
    }
    let cur = indices
        .iter()
        .position(|&i| i == state.model_idx)
        .unwrap_or(0);
    let next = if delta < 0 {
        cur.saturating_sub(1)
    } else {
        (cur + 1).min(indices.len() - 1)
    };
    state.model_idx = indices[next];
    // Scroll tracks visible-row position (draw also recenters).
    if next < state.model_scroll {
        state.model_scroll = next;
    }
}

fn handle_search(state: &mut IntroState, key: Key) -> Option<IntroResult> {
    if (key.mods.ctrl && matches!(key.code, KeyCode::Char('v') | KeyCode::Char('V')))
        || matches!(key.code, KeyCode::Insert)
    {
        if state.search_choice() != SearchChoice::Skip {
            state.search_focus = SearchFocus::Key;
            paste_from_clipboard(state);
        }
        return None;
    }

    if !key.mods.ctrl
        && !key.mods.alt
        && matches!(
            key.code,
            KeyCode::Char('u') | KeyCode::Char('U') | KeyCode::Char('s') | KeyCode::Char('S')
        )
        && state.search_focus == SearchFocus::List
        && state.can_use_existing_search()
    {
        state.step = Step::Done;
        return None;
    }

    match key.code {
        KeyCode::Left | KeyCode::Esc => {
            state.step = Step::Models;
            None
        }
        KeyCode::Up => {
            // Always move the choice, even when key field is focused.
            if state.search_idx > 0 {
                state.search_idx -= 1;
            }
            None
        }
        KeyCode::Down => {
            if state.search_idx < 2 {
                state.search_idx += 1;
            }
            None
        }
        KeyCode::Tab => {
            state.search_focus = match state.search_focus {
                SearchFocus::List => SearchFocus::Key,
                SearchFocus::Key => SearchFocus::List,
            };
            None
        }
        KeyCode::Enter | KeyCode::Right => match state.search_focus {
            SearchFocus::List => {
                if state.search_choice() == SearchChoice::Skip {
                    state.step = Step::Done;
                } else {
                    state.search_focus = SearchFocus::Key;
                }
                None
            }
            SearchFocus::Key => {
                // Key optional — allow empty and continue.
                state.step = Step::Done;
                None
            }
        },
        KeyCode::Backspace if state.search_focus == SearchFocus::Key => {
            state.search_key.pop();
            None
        }
        KeyCode::Char(c) if !key.mods.ctrl && !key.mods.alt => {
            if state.search_focus == SearchFocus::List
                && state.search_choice() != SearchChoice::Skip
            {
                state.search_focus = SearchFocus::Key;
            }
            if state.search_focus == SearchFocus::Key {
                state.search_key.push(c);
            }
            None
        }
        _ => None,
    }
}

fn handle_done(state: &mut IntroState, key: Key) -> Option<IntroResult> {
    match key.code {
        KeyCode::Left => {
            state.step = Step::Search;
            None
        }
        KeyCode::Enter | KeyCode::Right => match state.to_draft() {
            Some(draft) => Some(IntroResult::Completed(Box::new(draft))),
            None => {
                state.status = "setup incomplete".into();
                None
            }
        },
        KeyCode::Esc => None,
        _ => None,
    }
}

/// Advance helpers used by unit tests (no terminal).
#[cfg(test)]
mod step_tests {
    use super::*;
    use comb::KeyMods;

    fn opts() -> IntroOpts {
        IntroOpts {
            version: "0.1.0".into(),
            prefill: None,
        }
    }

    #[test]
    fn intro_always_starts_at_welcome() {
        let s = IntroState::new(IntroOpts {
            version: "0.1.0".into(),
            prefill: Some(IntroPrefill {
                provider_api_key: "sk-keep".into(),
                default_id: "gpt-4o".into(),
                ..Default::default()
            }),
        });
        assert_eq!(s.step, Step::Welcome);
        assert!(s.can_use_existing_provider());
    }

    #[test]
    fn welcome_enter_goes_to_modes() {
        let mut s = IntroState::new(opts());
        assert_eq!(s.step, Step::Welcome);
        let r = handle_key(
            &mut s,
            Key {
                code: KeyCode::Enter,
                mods: KeyMods::NONE,
            },
        );
        assert!(r.is_none());
        assert_eq!(s.step, Step::Modes);
    }

    #[test]
    fn modes_enter_goes_to_provider() {
        let mut s = IntroState::new(opts());
        s.step = Step::Modes;
        handle_key(
            &mut s,
            Key {
                code: KeyCode::Enter,
                mods: KeyMods::NONE,
            },
        );
        assert_eq!(s.step, Step::Provider);
    }

    fn ready_cards(state: &mut IntroState, cards: Vec<ModelCard>) {
        state.apply_cards(cards);
    }

    fn card(id: &str, name: &str, vision: bool) -> ModelCard {
        ModelCard {
            id: id.into(),
            name: name.into(),
            vision,
            video: false,
            audio: false,
            reasoning: false,
            tools: true,
            context: 128_000,
            enriched: true,
            cost_input: 0.0,
            cost_output: 0.0,
        }
    }

    #[test]
    fn models_type_filters_list() {
        let mut s = IntroState::new(opts());
        s.step = Step::Models;
        ready_cards(
            &mut s,
            vec![
                card("openai/gpt-4o", "GPT-4o", true),
                card("openai/gpt-4o-mini", "GPT-4o mini", true),
                card("anthropic/claude-sonnet", "Claude Sonnet", false),
            ],
        );
        handle_key(
            &mut s,
            Key {
                code: KeyCode::Char('c'),
                mods: KeyMods::NONE,
            },
        );
        handle_key(
            &mut s,
            Key {
                code: KeyCode::Char('l'),
                mods: KeyMods::NONE,
            },
        );
        assert_eq!(s.model_filter, "cl");
        let vis = s.visible_model_indices();
        assert_eq!(vis.len(), 1);
        if let FetchState::Ready(cards) = &s.fetch {
            assert!(cards[vis[0]].id.contains("claude"));
        } else {
            panic!("expected ready");
        }
    }

    #[test]
    fn models_esc_clears_filter_before_back() {
        let mut s = IntroState::new(opts());
        s.step = Step::Models;
        ready_cards(&mut s, vec![card("a/b", "B", false)]);
        s.model_filter = "b".into();
        handle_key(
            &mut s,
            Key {
                code: KeyCode::Esc,
                mods: KeyMods::NONE,
            },
        );
        assert_eq!(s.step, Step::Models);
        assert!(s.model_filter.is_empty());
        handle_key(
            &mut s,
            Key {
                code: KeyCode::Esc,
                mods: KeyMods::NONE,
            },
        );
        assert_eq!(s.step, Step::ProviderKey);
    }

    #[test]
    fn models_enter_then_right_advances() {
        let mut s = IntroState::new(opts());
        s.step = Step::Models;
        ready_cards(
            &mut s,
            vec![
                card("fast-flash", "Flash", false),
                card("smart-big", "Big", false),
                card("see", "See", true),
            ],
        );
        s.model = RoleAssignment::empty();
        s.model_idx = 0;
        handle_key(
            &mut s,
            Key {
                code: KeyCode::Enter,
                mods: KeyMods::NONE,
            },
        );
        assert_eq!(s.model.id, "fast-flash");
        assert_eq!(s.step, Step::Models);
        handle_key(
            &mut s,
            Key {
                code: KeyCode::Right,
                mods: KeyMods::NONE,
            },
        );
        assert_eq!(s.step, Step::Search);
    }

    #[test]
    fn models_right_continues_when_model_ready() {
        let mut s = IntroState::new(opts());
        s.step = Step::Models;
        ready_cards(
            &mut s,
            vec![
                card("def-model", "Def", false),
                card("smart-model", "Smart", false),
            ],
        );
        // apply_cards fills model — → should leave Models.
        assert!(s.models_ready());
        handle_key(
            &mut s,
            Key {
                code: KeyCode::Right,
                mods: KeyMods::NONE,
            },
        );
        assert_eq!(s.step, Step::Search);
    }

    #[test]
    fn models_right_stays_without_model() {
        let mut s = IntroState::new(opts());
        s.step = Step::Models;
        ready_cards(&mut s, vec![card("def-model", "Def", false)]);
        s.model = RoleAssignment::empty();
        assert!(!s.models_ready());
        handle_key(
            &mut s,
            Key {
                code: KeyCode::Right,
                mods: KeyMods::NONE,
            },
        );
        assert_eq!(s.step, Step::Models);
    }

    #[test]
    fn models_use_existing_while_loading_not_when_list_ready() {
        let mut s = IntroState::new(IntroOpts {
            version: "0.1.0".into(),
            prefill: Some(IntroPrefill {
                provider_base_url: "https://api.openai.com/v1".into(),
                provider_api_key_env: "OPENAI_API_KEY".into(),
                provider_api_key: "sk-keep".into(),
                default_id: "gpt-4o".into(),
                default_name: "GPT-4o".into(),
                search_backend: SearchBackend::None,
                search_api_key: None,
            }),
        });
        s.step = Step::Models;
        s.fetch = FetchState::Loading;
        assert!(s.can_use_existing_models());
        handle_key(
            &mut s,
            Key {
                code: KeyCode::Char('u'),
                mods: KeyMods::NONE,
            },
        );
        assert_eq!(s.step, Step::Search);

        let mut s2 = IntroState::new(opts());
        s2.step = Step::Models;
        ready_cards(&mut s2, vec![card("x", "X", false)]);
        assert!(!s2.can_use_existing_models());
        handle_key(
            &mut s2,
            Key {
                code: KeyCode::Char('s'),
                mods: KeyMods::NONE,
            },
        );
        assert_eq!(s2.step, Step::Models);
        assert_eq!(s2.model_filter, "s");
    }

    #[test]
    fn apply_cards_preserves_prefilled_model() {
        let mut s = IntroState::new(IntroOpts {
            version: "0.1.0".into(),
            prefill: Some(IntroPrefill {
                provider_api_key: "sk".into(),
                default_id: "keep-me".into(),
                default_name: "Keep".into(),
                ..Default::default()
            }),
        });
        ready_cards(
            &mut s,
            vec![
                card("other", "Other", false),
                card("keep-me", "Keep Renamed", false),
            ],
        );
        assert_eq!(s.model.id, "keep-me");
        assert_eq!(s.model.name, "Keep Renamed");
        assert!(matches!(s.fetch, FetchState::Ready(_)));
        if let FetchState::Ready(cards) = &s.fetch {
            assert_eq!(cards[s.model_idx].id, "keep-me");
        }
    }

    #[tokio::test]
    async fn advance_from_provider_always_fetches() {
        let mut s = IntroState::new(IntroOpts {
            version: "0.1.0".into(),
            prefill: Some(IntroPrefill {
                provider_base_url: "https://api.openai.com/v1".into(),
                provider_api_key: "sk-keep".into(),
                default_id: "gpt-4o".into(),
                default_name: "GPT-4o".into(),
                ..Default::default()
            }),
        });
        assert!(s.models_ready());
        advance_from_provider(&mut s);
        assert_eq!(s.step, Step::Models);
        assert!(matches!(s.fetch, FetchState::Loading));
        s.fetch_rx = None;
    }

    #[tokio::test]
    async fn provider_key_enter_starts_models_when_key_set() {
        let mut s = IntroState::new(opts());
        s.step = Step::ProviderKey;
        s.provider_key_focus = ProviderKeyFocus::Key;
        s.provider_key = "sk-test".into();
        let r = handle_key(
            &mut s,
            Key {
                code: KeyCode::Enter,
                mods: KeyMods::NONE,
            },
        );
        assert!(r.is_none());
        assert_eq!(s.step, Step::Models);
        assert!(matches!(s.fetch, FetchState::Loading));
        // Drop the in-flight fetch so the test runtime can shut down cleanly.
        s.fetch_rx = None;
    }

    #[test]
    fn provider_enter_opens_key_step() {
        let mut s = IntroState::new(opts());
        s.step = Step::Provider;
        s.provider_idx = 2;
        handle_key(
            &mut s,
            Key {
                code: KeyCode::Enter,
                mods: KeyMods::NONE,
            },
        );
        assert_eq!(s.step, Step::ProviderKey);
        assert_eq!(s.provider_idx, 2);
        assert_eq!(s.provider_key_focus, ProviderKeyFocus::Key);
    }

    #[test]
    fn provider_up_on_list_changes_selection() {
        let mut s = IntroState::new(opts());
        s.step = Step::Provider;
        s.provider_idx = 2;
        handle_key(
            &mut s,
            Key {
                code: KeyCode::Up,
                mods: KeyMods::NONE,
            },
        );
        assert_eq!(s.provider_idx, 1);
        assert_eq!(s.step, Step::Provider);
    }

    #[test]
    fn typing_u_without_key_does_not_use_existing() {
        let mut s = IntroState::new(opts());
        s.step = Step::Provider;
        handle_key(
            &mut s,
            Key {
                code: KeyCode::Char('u'),
                mods: KeyMods::NONE,
            },
        );
        // Without a key, 'u' on the list does nothing (no use-existing, no insert).
        assert_eq!(s.step, Step::Provider);
        assert!(s.provider_key.is_empty());
    }

    #[tokio::test]
    async fn use_existing_provider_keeps_key_and_models() {
        let mut s = IntroState::new(IntroOpts {
            version: "0.1.0".into(),
            prefill: Some(IntroPrefill {
                provider_base_url: "https://api.openai.com/v1".into(),
                provider_api_key_env: "OPENAI_API_KEY".into(),
                provider_api_key: "sk-keep".into(),
                default_id: "gpt-4o".into(),
                default_name: "GPT-4o".into(),
                search_backend: SearchBackend::None,
                search_api_key: None,
            }),
        });
        s.step = Step::Provider;
        assert!(s.can_use_existing_provider());
        handle_key(
            &mut s,
            Key {
                code: KeyCode::Char('u'),
                mods: KeyMods::NONE,
            },
        );
        assert_eq!(s.step, Step::Models);
        assert_eq!(s.provider_key, "sk-keep");
        assert_eq!(s.model.id, "gpt-4o");
        assert!(matches!(s.fetch, FetchState::Loading));
        s.fetch_rx = None;
    }

    #[test]
    fn left_from_key_returns_to_list_keeps_key() {
        let mut s = IntroState::new(opts());
        s.step = Step::ProviderKey;
        s.provider_key_focus = ProviderKeyFocus::Key;
        s.provider_key = "sk-keep".into();
        handle_key(
            &mut s,
            Key {
                code: KeyCode::Left,
                mods: KeyMods::NONE,
            },
        );
        assert_eq!(s.step, Step::Provider);
        assert_eq!(s.provider_key, "sk-keep");
    }

    #[test]
    fn paste_on_list_opens_key_step() {
        let mut s = IntroState::new(opts());
        s.step = Step::Provider;
        handle_paste(&mut s, " sk-abc \n");
        assert_eq!(s.step, Step::ProviderKey);
        assert_eq!(s.provider_key_focus, ProviderKeyFocus::Key);
        assert_eq!(s.provider_key, "sk-abc");
    }

    #[test]
    fn custom_enter_starts_on_url_field() {
        let mut s = IntroState::new(opts());
        s.step = Step::Provider;
        s.provider_idx = PRESETS
            .iter()
            .position(|p| p.is_custom)
            .expect("custom preset");
        handle_key(
            &mut s,
            Key {
                code: KeyCode::Enter,
                mods: KeyMods::NONE,
            },
        );
        assert_eq!(s.step, Step::ProviderKey);
        assert_eq!(s.provider_key_focus, ProviderKeyFocus::CustomUrl);
    }
}
