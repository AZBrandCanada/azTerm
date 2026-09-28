#!/usr/bin/env bash
set -euo pipefail

[ -f Cargo.toml ] || { echo "error: run from project root"; exit 1; }

python3 - << 'PY_EOF'
import re, sys

# ---------------------------------------------------------------- main.rs ---
path = "src/main.rs"
with open(path) as f:
    src = f.read()

# 1) Add last_heartbeat field to AppState
old_struct = """    pub toast_message: Option<(String, std::time::Instant)>,
"""
new_struct = """    pub toast_message: Option<(String, std::time::Instant)>,
    pub last_heartbeat: std::time::Instant,
"""
assert old_struct in src, "AppState field anchor not found"
if "last_heartbeat" not in src:
    src = src.replace(old_struct, new_struct, 1)

# 2) Initialize it in AppState::new
old_init = """            toast_message: None,
"""
new_init = """            toast_message: None,
            last_heartbeat: std::time::Instant::now(),
"""
assert old_init in src, "AppState init anchor not found"
if "last_heartbeat: std::time::Instant::now()" not in src:
    src = src.replace(old_init, new_init, 1)

# 3) Emit heartbeat at the top of update()
old_update_start = """    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Reconcile debug logging with current settings (cheap no-op if unchanged).
        debug_log::init(self.settings.debug_mode, &self.settings.debug_log_path);
"""
new_update_start = """    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Reconcile debug logging with current settings (cheap no-op if unchanged).
        debug_log::init(self.settings.debug_mode, &self.settings.debug_log_path);

        // Heartbeat: proves the UI thread is still ticking. If the app
        // ever locks up, the log will simply stop emitting these every
        // 5 seconds — that timestamp is where the freeze began.
        if self.settings.debug_mode
            && self.last_heartbeat.elapsed() >= std::time::Duration::from_secs(5)
        {
            self.last_heartbeat = std::time::Instant::now();
            debug_log::log(format!(
                "heartbeat view={:?} sess={} ws={} modal={} dragging_pane={:?} dragging_tab={:?} sftp_modal={}",
                self.active_view,
                self.active_session_id,
                self.active_workspace_idx,
                modal_open_flag(self),
                self.dragging_pane_id,
                self.dragging_tab_idx,
                self.sftp.has_open_modal(),
            ));
        }
"""
assert old_update_start in src, "update() entry anchor not found"
if "Heartbeat: proves the UI thread" not in src:
    src = src.replace(old_update_start, new_update_start, 1)

# 4) Helper function so we can call it before `modal_open` is defined
if "fn modal_open_flag" not in src:
    helper = """
fn modal_open_flag(app: &AppState) -> bool {
    app.show_update_modal
        || app.show_profile_modal
        || app.show_keygen_modal
        || app.ssh_auth_modal.is_some()
        || app.sftp.has_open_modal()
}
"""
    # insert just before `impl AppState {`
    src = src.replace("impl AppState {\n", helper + "\nimpl AppState {\n", 1)

with open(path, "w") as f:
    f.write(src)
print("patched", path)


# ------------------------------------------------------------ terminal.rs ---
path = "src/terminal.rs"
with open(path) as f:
    src = f.read()

# Log every keyboard event that reaches handle_keyboard_events for the
# focus-sensitive keys, so we can correlate user input with any freeze.
old_key_arm = """                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } => {
                        if *key == egui::Key::Enter {"""
new_key_arm = """                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } => {
                        if matches!(
                            key,
                            egui::Key::ArrowUp
                                | egui::Key::ArrowDown
                                | egui::Key::ArrowLeft
                                | egui::Key::ArrowRight
                                | egui::Key::Tab
                                | egui::Key::Escape
                                | egui::Key::Enter
                                | egui::Key::Backspace
                                | egui::Key::PageUp
                                | egui::Key::PageDown
                                | egui::Key::Home
                                | egui::Key::End
                        ) {
                            crate::dbg_log!(
                                "kbd id={} key={:?} ctrl={} shift={} alt={}",
                                self.id,
                                key,
                                modifiers.ctrl,
                                modifiers.shift,
                                modifiers.alt
                            );
                        }
                        if *key == egui::Key::Enter {"""
assert old_key_arm in src, "key arm anchor not found"
if 'kbd id={} key={:?} ctrl={}' not in src:
    src = src.replace(old_key_arm, new_key_arm, 1)

# Log around clipboard operations so any hang in a copy/paste path is
# visible as "clipboard_copy begin" with no matching "end".
old_copy_sel = """                    } else if settings.copy_on_select {
                        let selected = self.extract_selected_text();
                        if !selected.trim().is_empty() {
                            set_system_clipboard_text(Some(ui.ctx()), &selected);
                            let line_count = selected.lines().count().max(1);
                            let preview = if selected.len() > 30 {"""
new_copy_sel = """                    } else if settings.copy_on_select {
                        let selected = self.extract_selected_text();
                        if !selected.trim().is_empty() {
                            crate::dbg_log!(
                                "clipboard_copy begin id={} bytes={}",
                                self.id,
                                selected.len()
                            );
                            set_system_clipboard_text(Some(ui.ctx()), &selected);
                            crate::dbg_log!("clipboard_copy end id={}", self.id);
                            let line_count = selected.lines().count().max(1);
                            let preview = if selected.len() > 30 {"""
assert old_copy_sel in src, "copy-on-select anchor not found"
if 'clipboard_copy begin' not in src:
    src = src.replace(old_copy_sel, new_copy_sel, 1)

# Same for the double-click word copy path.
old_word_copy = """                    if settings.copy_on_select {
                        let selected = self.extract_selected_text();
                        if !selected.trim().is_empty() {
                            set_system_clipboard_text(Some(ui.ctx()), &selected);
                            let preview = if selected.len() > 24 {"""
new_word_copy = """                    if settings.copy_on_select {
                        let selected = self.extract_selected_text();
                        if !selected.trim().is_empty() {
                            crate::dbg_log!(
                                "clipboard_word_copy begin id={} bytes={}",
                                self.id,
                                selected.len()
                            );
                            set_system_clipboard_text(Some(ui.ctx()), &selected);
                            crate::dbg_log!("clipboard_word_copy end id={}", self.id);
                            let preview = if selected.len() > 24 {"""
assert old_word_copy in src, "word-copy anchor not found"
if 'clipboard_word_copy begin' not in src:
    src = src.replace(old_word_copy, new_word_copy, 1)

# Log the right-click paste path.
old_rc_paste = """            if settings.paste_on_right_click && response.secondary_clicked() {
                if let Some(clip) = get_system_clipboard_text() {"""
new_rc_paste = """            if settings.paste_on_right_click && response.secondary_clicked() {
                crate::dbg_log!("clipboard_paste begin id={} (right-click)", self.id);
                if let Some(clip) = get_system_clipboard_text() {"""
assert old_rc_paste in src, "right-click paste anchor not found"
if 'clipboard_paste begin' not in src:
    src = src.replace(old_rc_paste, new_rc_paste, 1)

# Log the Ctrl+Shift+V / Shift+Insert paste path.
old_kbd_paste = """                        {
                            if let Some(clip) = get_system_clipboard_text() {
                                self.send_paste(&clip);
                            }
                            continue;
                        }"""
new_kbd_paste = """                        {
                            crate::dbg_log!(
                                "clipboard_paste begin id={} (kbd paste)",
                                self.id
                            );
                            if let Some(clip) = get_system_clipboard_text() {
                                self.send_paste(&clip);
                            }
                            crate::dbg_log!("clipboard_paste end id={}", self.id);
                            continue;
                        }"""
assert old_kbd_paste in src, "kbd paste anchor not found"
if 'clipboard_paste begin' not in src:
    src = src.replace(old_kbd_paste, new_kbd_paste, 1)

with open(path, "w") as f:
    f.write(src)
print("patched", path)
PY_EOF

echo
echo "Done. Rebuild:  cargo build --release"
