//! Bangla IME UI state, keyboard interception, and phonetic candidate popup.
//!
//! Handles typing in Avro Phonetic, Jatiya / National, and Bijoy ANSI (SutonnyMJ) modes,
//! complete with hotkeys (F12, Ctrl+Alt+B, Ctrl+Alt+V), cluster reordering, and candidate suggestions.

use egui::{Context, Key, Modifiers, RichText};
use gridcraft_engine::cmd::bangla::{
    BanglaMode, bijoy_ansi_key, bijoy_linker_vowel, find_bijoy_conjunct, jatiya_key, jatiya_linker_vowel, phonetic_candidates, phonetic_transliterate,
};
use serde_json::json;

use crate::SheetApp;
use crate::editor::EditState;
use crate::theme;

#[derive(Clone, Debug, Default)]
pub struct BanglaUiState {
    // Phonetic mode state
    pub phonetic_buffer: String,
    pub phonetic_candidates: Vec<String>,
    pub phonetic_rendered_len: usize,
    pub candidate_selected: usize,

    // Jatiya mode state
    pub jatiya_linker: bool,
    pub jatiya_pre_kar: Option<&'static str>,
    pub jatiya_reph: bool,

    // ANSI mode state
    pub ansi_linker: bool,
    pub ansi_prev_bn: Option<char>,
    pub ansi_prev_ansi_len: usize,
    pub ansi_reph: bool,
}

impl BanglaUiState {
    pub fn reset(&mut self) {
        self.phonetic_buffer.clear();
        self.phonetic_candidates.clear();
        self.phonetic_rendered_len = 0;
        self.candidate_selected = 0;
        self.jatiya_linker = false;
        self.jatiya_pre_kar = None;
        self.jatiya_reph = false;
        self.ansi_linker = false;
        self.ansi_prev_bn = None;
        self.ansi_prev_ansi_len = 0;
        self.ansi_reph = false;
    }
}

/// Safely inserts a string at the editor's caret position.
pub fn insert_text(ed: &mut EditState, s: &str) {
    let char_indices: Vec<(usize, char)> = ed.text.char_indices().collect();
    let byte_pos =
        if ed.caret >= char_indices.len() { ed.text.len() } else { char_indices.get(ed.caret).map(|(pos, _)| *pos).unwrap_or(ed.text.len()) };
    ed.text.insert_str(byte_pos, s);
    ed.caret += s.chars().count();
    ed.sync_caret = true;
}

/// Safely removes `count` characters immediately before the caret.
pub fn backspace_chars(ed: &mut EditState, count: usize) {
    for _ in 0..count {
        if ed.caret == 0 {
            break;
        }
        let char_indices: Vec<(usize, char)> = ed.text.char_indices().collect();
        if ed.caret <= char_indices.len() {
            let start_byte = char_indices.get(ed.caret - 1).map(|(pos, _)| *pos).unwrap_or(0);
            let end_byte =
                if ed.caret < char_indices.len() { char_indices.get(ed.caret).map(|(pos, _)| *pos).unwrap_or(ed.text.len()) } else { ed.text.len() };
            if start_byte <= end_byte && end_byte <= ed.text.len() {
                ed.text.drain(start_byte..end_byte);
                ed.caret = ed.caret.saturating_sub(1);
                ed.sync_caret = true;
            }
        }
    }
}

/// Returns the slice of text before the caret.
pub fn text_before_caret(ed: &EditState) -> &str {
    let char_indices: Vec<(usize, char)> = ed.text.char_indices().collect();
    if ed.caret == 0 {
        ""
    } else if ed.caret >= char_indices.len() {
        &ed.text
    } else if let Some(&(byte_pos, _)) = char_indices.get(ed.caret) {
        ed.text.get(..byte_pos).unwrap_or("")
    } else {
        ""
    }
}

/// Dispatches Bangla-specific shortcut keys (F12, Ctrl+Alt+B, Ctrl+Alt+V, and phonetic suggestions).
/// Returns `true` if the key was handled.
pub fn handle_key_event(app: &mut SheetApp, key: Key, modifiers: Modifiers) -> bool {
    // F12: Toggle Bangla Mode
    if key == Key::F12 && !modifiers.shift && !modifiers.alt && !modifiers.ctrl && !modifiers.command {
        let _ = app.run("tools.banglaMode", json!({"toggle": true}));
        app.bangla.reset();
        return true;
    }

    // Ctrl+Alt+B / Cmd+Alt+B: Switch to ANSI mode
    // Ctrl+Alt+V / Cmd+Alt+V: Switch to Jatiya mode
    if (key == Key::B || key == Key::V) && (modifiers.ctrl || modifiers.command) && modifiers.alt {
        let mode = if key == Key::B { "ansi" } else { "jatiya" };
        let _ = app.run("tools.banglaMode", json!({"mode": mode}));
        app.bangla.reset();
        return true;
    }

    // Candidate selection in Phonetic mode
    if app.session.bangla_mode == BanglaMode::Phonetic && !app.bangla.phonetic_candidates.is_empty() {
        let candidate_idx = match key {
            Key::Num1 => Some(0),
            Key::Num2 => Some(1),
            Key::Num3 => Some(2),
            Key::Num4 => Some(3),
            Key::Num5 => Some(4),
            _ => None,
        };

        if let Some(idx) = candidate_idx
            && let Some(cand) = app.bangla.phonetic_candidates.get(idx).cloned()
            && let Some(ed) = app.editor.as_mut()
        {
            backspace_chars(ed, app.bangla.phonetic_rendered_len);
            insert_text(ed, &cand);
            app.bangla.reset();
            return true;
        }

        if (key == Key::Space || key == Key::Enter)
            && let Some(cand) = app.bangla.phonetic_candidates.get(app.bangla.candidate_selected).cloned()
            && let Some(ed) = app.editor.as_mut()
        {
            backspace_chars(ed, app.bangla.phonetic_rendered_len);
            insert_text(ed, &cand);
            if key == Key::Space {
                insert_text(ed, " ");
            }
            app.bangla.reset();
            return true;
        }

        if key == Key::Backspace
            && !app.bangla.phonetic_buffer.is_empty()
            && let Some(ed) = app.editor.as_mut()
        {
            app.bangla.phonetic_buffer.pop();
            backspace_chars(ed, app.bangla.phonetic_rendered_len);
            if app.bangla.phonetic_buffer.is_empty() {
                app.bangla.phonetic_candidates.clear();
                app.bangla.phonetic_rendered_len = 0;
            } else {
                app.bangla.phonetic_candidates = phonetic_candidates(&app.bangla.phonetic_buffer);
                let replacement =
                    app.bangla.phonetic_candidates.first().cloned().unwrap_or_else(|| phonetic_transliterate(&app.bangla.phonetic_buffer));
                insert_text(ed, &replacement);
                app.bangla.phonetic_rendered_len = replacement.chars().count();
            }
            return true;
        }
    }

    // Jatiya mode backspace: discard armed linker or pre-kar
    if app.session.bangla_mode == BanglaMode::Jatiya && key == Key::Backspace {
        if app.bangla.jatiya_pre_kar.is_some() {
            app.bangla.jatiya_pre_kar = None;
            return true;
        }
        if app.bangla.jatiya_linker {
            app.bangla.jatiya_linker = false;
            return true;
        }
    }

    false
}

/// Handles character input according to active Bangla mode.
/// Returns `true` if text was processed.
pub fn handle_text_event(app: &mut SheetApp, text: &str) -> bool {
    if app.session.bangla_mode == BanglaMode::Off || text.is_empty() {
        return false;
    }

    // If no editor is open, open one on the active cell
    if app.editor.is_none() {
        app.begin_edit(Some(String::new()), false);
    }

    let Some(ed) = app.editor.as_mut() else {
        return false;
    };

    match app.session.bangla_mode {
        BanglaMode::Off => false,
        BanglaMode::Jatiya => {
            for c in text.chars() {
                process_jatiya_char(ed, &mut app.bangla, c);
            }
            true
        }
        BanglaMode::Ansi => {
            for c in text.chars() {
                process_ansi_char(ed, &mut app.bangla, c);
            }
            true
        }
        BanglaMode::Phonetic => {
            for c in text.chars() {
                process_phonetic_char(ed, &mut app.bangla, c);
            }
            true
        }
    }
}

fn process_jatiya_char(ed: &mut EditState, bangla: &mut BanglaUiState, c: char) {
    // 1. Linker key 'g'
    if c == 'g' {
        if bangla.jatiya_linker {
            insert_text(ed, "্");
            bangla.jatiya_linker = false;
        } else {
            bangla.jatiya_linker = true;
            let before = text_before_caret(ed);
            let prev_char = before.chars().next_back();
            if let Some(kar @ ('ি' | 'ে' | 'ৈ')) = prev_char {
                backspace_chars(ed, 1);
                insert_text(ed, "্");
                bangla.jatiya_pre_kar = Some(match kar {
                    'ি' => "ি",
                    'ে' => "ে",
                    'ৈ' => "ৈ",
                    _ => "",
                });
            } else {
                insert_text(ed, "্");
            }
        }
        return;
    }

    // 2. If linker 'g' was active
    if bangla.jatiya_linker {
        bangla.jatiya_linker = false;
        if let Some(vowel) = jatiya_linker_vowel(c) {
            let before = text_before_caret(ed);
            if before.ends_with('্') {
                backspace_chars(ed, 1);
            }
            if let Some(stashed) = bangla.jatiya_pre_kar.take() {
                insert_text(ed, stashed);
            }
            insert_text(ed, vowel);
            return;
        }
        if c == '|' {
            let before = text_before_caret(ed);
            if before.ends_with('্') {
                backspace_chars(ed, 1);
            }
            if let Some(stashed) = bangla.jatiya_pre_kar.take() {
                insert_text(ed, stashed);
            }
            insert_text(ed, "ঃ");
            return;
        }
        if c == '.' || c == 'G' {
            let before = text_before_caret(ed);
            if before.ends_with('্') {
                backspace_chars(ed, 1);
            }
            if let Some(stashed) = bangla.jatiya_pre_kar.take() {
                insert_text(ed, stashed);
            }
            insert_text(ed, "।");
            return;
        }
    }

    // 3. Reph key 'A'
    if c == 'A' {
        bangla.jatiya_reph = true;
        return;
    }

    // 4. Pre-kar keys 'd' (ি), 'c' (ে), 'C' (ৈ)
    if c == 'd' || c == 'c' || c == 'C' {
        let kar = match c {
            'd' => "ি",
            'c' => "ে",
            'C' => "ৈ",
            _ => "",
        };
        bangla.jatiya_pre_kar = Some(kar);
        return;
    }

    // 5. O-kar / Ou-kar composition: 'c' (ে) + consonant + 'f' (া) = 'ো'
    if c == 'f' || c == 'X' {
        let before = text_before_caret(ed);
        if before.ends_with('ে') {
            backspace_chars(ed, 1);
            let kar = if c == 'f' { "ো" } else { "ৌ" };
            insert_text(ed, kar);
            return;
        }
    }

    // 6. Stop mark / Dari: '.' or 'G'
    if c == 'G' {
        if let Some(kar) = bangla.jatiya_pre_kar.take() {
            insert_text(ed, kar);
        }
        insert_text(ed, "।");
        return;
    }
    if c == '.' {
        if let Some(kar) = bangla.jatiya_pre_kar.take() {
            insert_text(ed, kar);
        }
        let before = text_before_caret(ed);
        let prev_char = before.chars().next_back();
        if let Some(ch) = prev_char {
            if ch.is_ascii_digit() || ('\u{09E6}'..='\u{09EF}').contains(&ch) {
                insert_text(ed, ".");
            } else {
                insert_text(ed, "।");
            }
        } else {
            insert_text(ed, "।");
        }
        return;
    }

    // 7. Bisarga: '|'
    if c == '|' {
        if let Some(kar) = bangla.jatiya_pre_kar.take() {
            insert_text(ed, kar);
        }
        insert_text(ed, "ঃ");
        return;
    }

    // 7.5. Ra-fala ('z' = "্র") and Ya-fala ('Z' = "্য")
    if c == 'z' || c == 'Z' {
        let fala = if c == 'z' { "্র" } else { "্য" };
        let before = text_before_caret(ed);
        let prev_char = before.chars().next_back();
        let mut moved_kar: Option<char> = None;

        if let Some(kar @ ('ি' | 'ে' | 'ৈ' | 'া' | 'ী' | 'ু' | 'ূ' | 'ৃ' | 'ো' | 'ৌ')) = prev_char {
            backspace_chars(ed, 1);
            moved_kar = Some(kar);
        }

        let before2 = text_before_caret(ed);
        if before2.ends_with('্') {
            backspace_chars(ed, 1);
        }

        let mut out = String::new();
        if bangla.jatiya_reph {
            out.push_str("র্");
            bangla.jatiya_reph = false;
        }
        out.push_str(fala);
        if let Some(kar) = moved_kar {
            out.push(kar);
        }
        if let Some(stashed_kar) = bangla.jatiya_pre_kar.take() {
            out.push_str(stashed_kar);
        }
        insert_text(ed, &out);
        return;
    }

    // 8. Normal character lookup
    if let Some(s) = jatiya_key(c) {
        let mut out = String::new();
        if bangla.jatiya_reph {
            out.push_str("র্");
            bangla.jatiya_reph = false;
        }
        out.push_str(s);
        if let Some(kar) = bangla.jatiya_pre_kar.take() {
            out.push_str(kar);
        }
        insert_text(ed, &out);
    } else {
        if let Some(kar) = bangla.jatiya_pre_kar.take() {
            insert_text(ed, kar);
        }
        if bangla.jatiya_reph {
            insert_text(ed, "র্");
            bangla.jatiya_reph = false;
        }
        insert_text(ed, &c.to_string());
    }
}

fn process_ansi_char(ed: &mut EditState, bangla: &mut BanglaUiState, c: char) {
    // 1. Linker key 'g'
    if c == 'g' {
        if bangla.ansi_linker {
            insert_text(ed, "&");
            bangla.ansi_linker = false;
            bangla.ansi_prev_bn = None;
        } else {
            bangla.ansi_linker = true;
        }
        return;
    }

    // 2. If linker active
    if bangla.ansi_linker {
        bangla.ansi_linker = false;
        if let Some(vowel) = bijoy_linker_vowel(c) {
            insert_text(ed, vowel);
            bangla.ansi_prev_bn = None;
            bangla.ansi_prev_ansi_len = vowel.chars().count();
            return;
        }
        if let Some(c1) = bangla.ansi_prev_bn.take()
            && let Some(c2_str) = jatiya_key(c)
            && let Some(c2) = c2_str.chars().next()
            && let Some(conj) = find_bijoy_conjunct(c1, c2)
        {
            backspace_chars(ed, bangla.ansi_prev_ansi_len);
            insert_text(ed, conj);
            bangla.ansi_prev_ansi_len = conj.chars().count();
            return;
        }
    }

    // 3. Reph key 'A'
    if c == 'A' {
        insert_text(ed, "©");
        bangla.ansi_prev_bn = None;
        bangla.ansi_prev_ansi_len = 1;
        return;
    }

    // 4. Ra-fala 'z'
    if c == 'z' {
        let before = text_before_caret(ed);
        let prev_char = before.chars().next_back();
        if let Some(c1) = prev_char {
            let fala_repl = match c1 {
                'c' => Some("cÖ"), // প + র-ফলা
                'K' => Some("µ"),  // ক + র-ফলা
                'f' => Some("Î"),  // ত + র-ফলা
                'i' => Some("kÖ"), // শ + র-ফলা
                _ => None,
            };
            if let Some(repl) = fala_repl {
                backspace_chars(ed, 1);
                insert_text(ed, repl);
                bangla.ansi_prev_bn = None;
                bangla.ansi_prev_ansi_len = repl.chars().count();
                return;
            }
        }
        insert_text(ed, "ª");
        bangla.ansi_prev_bn = None;
        bangla.ansi_prev_ansi_len = 1;
        return;
    }

    // 5. Normal ANSI key
    if let Some(s) = bijoy_ansi_key(c) {
        let mapped = if c == 'c' {
            let before = text_before_caret(ed);
            let prev = before.chars().next_back();
            if prev.is_none() || prev.is_some_and(|ch| ch.is_whitespace() || ch == '(' || ch == '\"') { "†" } else { "‡" }
        } else {
            s
        };
        insert_text(ed, mapped);
        bangla.ansi_prev_bn = jatiya_key(c).and_then(|s| s.chars().next());
        bangla.ansi_prev_ansi_len = mapped.chars().count();
    } else {
        insert_text(ed, &c.to_string());
        bangla.ansi_prev_bn = None;
        bangla.ansi_prev_ansi_len = 1;
    }
}

fn process_phonetic_char(ed: &mut EditState, bangla: &mut BanglaUiState, c: char) {
    let is_word_char = c.is_ascii_alphabetic() || c == '\'' || c == '^' || c == '`';

    if !is_word_char {
        // Delimiter commits composition
        bangla.phonetic_buffer.clear();
        bangla.phonetic_candidates.clear();
        bangla.phonetic_rendered_len = 0;

        let delim = match c {
            '|' => "।",
            '$' => "৳",
            _ => {
                insert_text(ed, &c.to_string());
                return;
            }
        };
        insert_text(ed, delim);
        return;
    }

    bangla.phonetic_buffer.push(c);
    bangla.phonetic_candidates = phonetic_candidates(&bangla.phonetic_buffer);
    let best = bangla.phonetic_candidates.first().cloned().unwrap_or_else(|| phonetic_transliterate(&bangla.phonetic_buffer));

    backspace_chars(ed, bangla.phonetic_rendered_len);
    insert_text(ed, &best);
    bangla.phonetic_rendered_len = best.chars().count();
    bangla.candidate_selected = 0;
}

/// Renders the floating phonetic candidate suggestions popup right below the active cell editor.
pub fn show_candidate_popup(app: &mut SheetApp, ctx: &Context) {
    if app.session.bangla_mode != BanglaMode::Phonetic || app.bangla.phonetic_candidates.is_empty() {
        return;
    }

    if app.editor.is_none() {
        return;
    }

    let popup_pos = ctx.input(|i| i.pointer.hover_pos().unwrap_or_else(|| egui::pos2(200.0, 100.0)));

    let t = theme::Tokens::get(ctx);
    let mut selected_cand: Option<String> = None;

    egui::Area::new(egui::Id::new("gridcraft.bangla_candidates")).order(egui::Order::Tooltip).fixed_pos(popup_pos + egui::vec2(10.0, 20.0)).show(
        ctx,
        |ui| {
            egui::Frame::popup(ui.style()).fill(t.window).stroke(egui::Stroke::new(1.0, t.separator)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("বাং Avro:").font(theme::ui_font(11.0)).color(t.text_dim));
                    for (i, cand) in app.bangla.phonetic_candidates.iter().take(5).enumerate() {
                        let label = format!("{}. {}", i + 1, cand);
                        let is_sel = i == app.bangla.candidate_selected;
                        let text = if is_sel {
                            RichText::new(label).font(theme::ui_font(13.0)).color(t.accent).strong()
                        } else {
                            RichText::new(label).font(theme::ui_font(12.5)).color(t.text)
                        };
                        if ui.selectable_label(is_sel, text).clicked() {
                            selected_cand = Some(cand.clone());
                        }
                    }
                });
            });
        },
    );

    if let Some(cand) = selected_cand
        && let Some(ed) = app.editor.as_mut()
    {
        backspace_chars(ed, app.bangla.phonetic_rendered_len);
        insert_text(ed, &cand);
        app.bangla.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gridcraft_engine::Session;
    use gridcraft_engine::core::CellRef;

    #[test]
    fn bangla_ui_state_reset() {
        let mut st = BanglaUiState::default();
        st.phonetic_buffer = "ami".into();
        st.phonetic_candidates = vec!["আমি".into()];
        st.phonetic_rendered_len = 3;
        st.jatiya_linker = true;
        st.ansi_linker = true;
        st.reset();
        assert!(st.phonetic_buffer.is_empty());
        assert!(st.phonetic_candidates.is_empty());
        assert_eq!(st.phonetic_rendered_len, 0);
        assert!(!st.jatiya_linker);
        assert!(!st.ansi_linker);
    }

    #[test]
    fn editor_text_manipulation_helpers() {
        let mut ed = EditState::new(0, CellRef::new(0, 0), "Hello World".into(), false, false);
        ed.caret = 5; // after "Hello"
        insert_text(&mut ed, " Beautiful");
        assert_eq!(ed.text, "Hello Beautiful World");
        assert_eq!(ed.caret, 15);
        assert!(ed.sync_caret);

        backspace_chars(&mut ed, 10);
        assert_eq!(ed.text, "Hello World");
        assert_eq!(ed.caret, 5);
    }

    #[test]
    fn phonetic_buffer_backspace() {
        let mut app = SheetApp::new(Session::default(), Default::default());
        app.session.bangla_mode = BanglaMode::Phonetic;
        let ed = EditState::new(0, CellRef::new(0, 0), "".into(), false, false);
        app.editor = Some(ed);

        // Type 'a', 'm', 'i'
        handle_text_event(&mut app, "a");
        assert_eq!(app.bangla.phonetic_buffer, "a");
        assert_eq!(app.editor.as_ref().unwrap().text, "আ");

        handle_text_event(&mut app, "m");
        assert_eq!(app.bangla.phonetic_buffer, "am");

        handle_text_event(&mut app, "i");
        assert_eq!(app.bangla.phonetic_buffer, "ami");
        assert_eq!(app.editor.as_ref().unwrap().text, "আমি");

        // Backspace removes one char from buffer and updates text
        let handled = handle_key_event(&mut app, Key::Backspace, Modifiers::default());
        assert!(handled);
        assert_eq!(app.bangla.phonetic_buffer, "am");
        assert_eq!(app.editor.as_ref().unwrap().text, "আম");
    }

    #[test]
    fn phonetic_space_commits_buffer() {
        let mut app = SheetApp::new(Session::default(), Default::default());
        app.session.bangla_mode = BanglaMode::Phonetic;
        let ed = EditState::new(0, CellRef::new(0, 0), "".into(), false, false);
        app.editor = Some(ed);

        handle_text_event(&mut app, "a");
        handle_text_event(&mut app, "m");
        handle_text_event(&mut app, "i");
        assert_eq!(app.editor.as_ref().unwrap().text, "আমি");

        let handled = handle_key_event(&mut app, Key::Space, Modifiers::default());
        assert!(handled);
        assert_eq!(app.editor.as_ref().unwrap().text, "আমি ");
        assert!(app.bangla.phonetic_buffer.is_empty());
    }

    #[test]
    fn jatiya_typing_and_conjunct() {
        let mut app = SheetApp::new(Session::default(), Default::default());
        app.session.bangla_mode = BanglaMode::Jatiya;
        let ed = EditState::new(0, CellRef::new(0, 0), "".into(), false, false);
        app.editor = Some(ed);

        // 'j' in Jatiya is 'ক'
        handle_text_event(&mut app, "j");
        assert_eq!(app.editor.as_ref().unwrap().text, "ক");

        // 'g' in Jatiya is linker হসন্ত
        handle_text_event(&mut app, "g");
        assert!(app.bangla.jatiya_linker);

        // 'j' again forms conjunct ক্ক
        handle_text_event(&mut app, "j");
        assert_eq!(app.editor.as_ref().unwrap().text, "ক্ক");
        assert!(!app.bangla.jatiya_linker);
    }

    #[test]
    fn bijoy_ansi_typing() {
        let mut app = SheetApp::new(Session::default(), Default::default());
        app.session.bangla_mode = BanglaMode::Ansi;
        let ed = EditState::new(0, CellRef::new(0, 0), "".into(), false, false);
        app.editor = Some(ed);

        // 'k' in Bijoy produces 'Z' (SutonnyMJ code for ত)
        handle_text_event(&mut app, "k");
        assert_eq!(app.editor.as_ref().unwrap().text, "Z");

        // 'j' in Bijoy produces 'K' (SutonnyMJ code for ক)
        handle_text_event(&mut app, "j");
        assert_eq!(app.editor.as_ref().unwrap().text, "ZK");
    }

    #[test]
    fn mode_toggles_f12_and_shortcuts() {
        let mut app = SheetApp::new(Session::default(), Default::default());
        assert_eq!(app.session.bangla_mode, BanglaMode::Off);

        // F12 cycles modes: Off -> Phonetic -> Jatiya -> Ansi -> Off
        handle_key_event(&mut app, Key::F12, Modifiers::default());
        assert_eq!(app.session.bangla_mode, BanglaMode::Phonetic);

        handle_key_event(&mut app, Key::F12, Modifiers::default());
        assert_eq!(app.session.bangla_mode, BanglaMode::Jatiya);

        handle_key_event(&mut app, Key::F12, Modifiers::default());
        assert_eq!(app.session.bangla_mode, BanglaMode::Ansi);

        handle_key_event(&mut app, Key::F12, Modifiers::default());
        assert_eq!(app.session.bangla_mode, BanglaMode::Off);

        // Ctrl+Alt+B sets Ansi
        let ctrl_alt = Modifiers { ctrl: true, alt: true, ..Default::default() };
        handle_key_event(&mut app, Key::B, ctrl_alt);
        assert_eq!(app.session.bangla_mode, BanglaMode::Ansi);

        // Ctrl+Alt+V sets Jatiya
        handle_key_event(&mut app, Key::V, ctrl_alt);
        assert_eq!(app.session.bangla_mode, BanglaMode::Jatiya);
    }
}
