// src/ui/settings_view.rs
use crate::db::Database;
use crate::settings::BackspaceSequence;
use crate::theme::{setting_row_disabled, setting_row_toggle, ThemeConfig};
use crate::{AppState, SettingsCategory};
use eframe::egui;

pub fn render_settings_view(app: &mut AppState, ctx: &egui::Context, ui: &mut egui::Ui) {
    ui.columns(2, |columns| {
        columns[0].set_max_width(210.0);
        columns[0].vertical(|ui| {
            ui.add_space(10.0);
            ui.label(egui::RichText::new("Preferences").strong().size(16.0).color(app.theme.text_primary_color()));
            ui.add_space(12.0);

            let nav_item = |ui: &mut egui::Ui, cat: SettingsCategory, label: &str, current: SettingsCategory, theme: &ThemeConfig| -> bool {
                let is_active = current == cat;

                let (fill, edge, text_color) = if is_active {
                    (
                        theme.accent_color(),
                        crate::modern::darken(theme.accent_color(), 55),
                        theme.on_accent_color(),
                    )
                } else {
                    (
                        theme.bg_card_color(),
                        crate::modern::darken(theme.bg_card_color(), 40),
                        theme.text_primary_color(),
                    )
                };

                crate::modern::Button3D::new(label)
                    .fill(fill)
                    .edge(edge)
                    .text_color(text_color)
                    .min_size(egui::vec2(180.0, 30.0))
                    .show(ui, theme)
                    .clicked()
            };

            if nav_item(ui, SettingsCategory::Appearance, "Themes & Window", app.settings_category, &app.theme) {
                app.settings_category = SettingsCategory::Appearance;
            }
            ui.add_space(4.0);
            if nav_item(ui, SettingsCategory::Terminal, "Terminal Interaction", app.settings_category, &app.theme) {
                app.settings_category = SettingsCategory::Terminal;
            }
            ui.add_space(4.0);
            if nav_item(ui, SettingsCategory::ShellEnv, "Shell & Environment", app.settings_category, &app.theme) {
                app.settings_category = SettingsCategory::ShellEnv;
            }
            ui.add_space(4.0);
            if nav_item(ui, SettingsCategory::Sftp, "SFTP & Transfers", app.settings_category, &app.theme) {
                app.settings_category = SettingsCategory::Sftp;
            }
            ui.add_space(4.0);
            if nav_item(ui, SettingsCategory::System, "Application & System", app.settings_category, &app.theme) {
                app.settings_category = SettingsCategory::System;
            }

            ui.add_space(20.0);
            ui.separator();
            ui.add_space(8.0);
            if crate::modern::button(ui, &app.theme, "Restore Defaults").clicked() {
                app.settings = crate::settings::AppSettings::default();
                app.settings.save();
                app.theme = ThemeConfig::default();
                Database::save_active_theme(&app.theme);
                ctx.set_zoom_factor(1.0);
                app.set_toast("Defaults Restored");
            }
        });

        columns[1].vertical(|ui| {
            ui.add_space(10.0);
            let mut changed = false;

            app.theme.card_frame().show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    match app.settings_category {
                        SettingsCategory::Appearance => {
                            ui.label(egui::RichText::new("Themes & Window Appearance").strong().size(16.0).color(app.theme.accent_color()));
                            ui.label(egui::RichText::new("Choose from built-in themes, customize colors, transparency, and window decorations.").small().color(app.theme.text_muted_color()));
                            ui.add_space(12.0);

                            if setting_row_toggle(
                                ui,
                                "Use OS Native Title Bar",
                                "Use system window decorations. Toggle off to use the sleek custom integrated AZTerm titlebar.",
                                &mut app.settings.use_system_titlebar,
                                &app.theme,
                            ) {
                                ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(app.settings.use_system_titlebar));
                                changed = true;
                            }

                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.label(egui::RichText::new("Window Zoom Level").strong().color(app.theme.text_primary_color()));
                                    ui.label(egui::RichText::new("Current scale (Ctrl +, Ctrl -, Ctrl 0 to reset).").small().color(app.theme.text_muted_color()));
                                });
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if crate::modern::button_small(ui, &app.theme, "Reset (100%)").clicked() {
                                        app.settings.zoom_factor = 1.0;
                                        ctx.set_zoom_factor(1.0);
                                        changed = true;
                                    }
                                    ui.label(format!("{}%", (app.settings.zoom_factor * 100.0).round() as u32));
                                });
                            });
                            ui.add_space(8.0);
                            ui.separator();
                            ui.add_space(8.0);

                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.label(egui::RichText::new("Window Background Opacity").strong().color(app.theme.text_primary_color()));
                                    ui.label(egui::RichText::new("Set terminal transparency (0% = fully transparent, 100% = opaque). Live preview as you drag.").small().color(app.theme.text_muted_color()));
                                });
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    let pct = (app.theme.opacity * 100.0).round() as u32;
                                    ui.label(format!("{}%", pct));
                                    if ui.add(egui::Slider::new(&mut app.theme.opacity, 0.0..=1.0).show_value(false)).changed() {
                                        Database::save_active_theme(&app.theme);
                                    }
                                });
                            });
                            ui.add_space(8.0);
                            ui.separator();
                            ui.add_space(8.0);

                            ui.label(egui::RichText::new("Select Theme Preset").strong().color(app.theme.text_primary_color()));
                            ui.add_space(4.0);

                            let builtins = ThemeConfig::builtins();
                            let mut selected_theme_to_apply: Option<ThemeConfig> = None;

                            ui.horizontal_wrapped(|ui| {
                                for preset in &builtins {
                                    let is_active = app.theme.id == preset.id;
                                    let (fill, edge, txt) = if is_active {
                                        (app.theme.accent_color(), crate::modern::darken(app.theme.accent_color(), 55), app.theme.on_accent_color())
                                    } else {
                                        (app.theme.bg_card_color(), crate::modern::darken(app.theme.bg_card_color(), 40), app.theme.text_primary_color())
                                    };
                                    let btn = crate::modern::Button3D::new(&preset.name)
                                        .small()
                                        .fill(fill).edge(edge).text_color(txt)
                                        .show(ui, &app.theme);
                                    if btn.clicked() {
                                        let mut new_th = preset.clone();
                                        new_th.opacity = app.theme.opacity;
                                        selected_theme_to_apply = Some(new_th);
                                    }
                                }
                                for custom in &app.custom_themes {
                                    let is_active = app.theme.id == custom.id;
                                    let label = format!("* {}", custom.name);
                                    let (fill, edge, txt) = if is_active {
                                        (app.theme.accent_color(), crate::modern::darken(app.theme.accent_color(), 55), app.theme.on_accent_color())
                                    } else {
                                        (app.theme.bg_card_color(), crate::modern::darken(app.theme.bg_card_color(), 40), app.theme.text_primary_color())
                                    };
                                    let btn = crate::modern::Button3D::new(&label)
                                        .small()
                                        .fill(fill).edge(edge).text_color(txt)
                                        .show(ui, &app.theme);
                                    if btn.clicked() {
                                        let mut new_th = custom.clone();
                                        new_th.opacity = app.theme.opacity;
                                        selected_theme_to_apply = Some(new_th);
                                    }
                                }
                            });

                            if let Some(new_th) = selected_theme_to_apply {
                                app.theme = new_th;
                                Database::save_active_theme(&app.theme);
                                app.set_toast(format!("Theme applied: {}", app.theme.name));
                            }

                            ui.add_space(8.0);
                            ui.horizontal(|ui| {
                                ui.text_edit_singleline(&mut app.new_theme_name);
                                if crate::modern::button(ui, &app.theme, "+ Duplicate Current as Custom").clicked() {
                                    let mut custom = app.theme.clone();
                                    custom.id = format!("custom_{}", chrono::Utc::now().timestamp_millis());
                                    custom.name = app.new_theme_name.clone();
                                    custom.is_builtin = false;
                                    app.custom_themes.push(custom.clone());
                                    app.theme = custom;
                                    Database::save_custom_themes(&app.custom_themes);
                                    Database::save_active_theme(&app.theme);
                                    app.set_toast("Custom Theme Created");
                                }

                                if !app.theme.is_builtin {
                                    if crate::modern::button_danger(ui, &app.theme, "Delete This Custom Theme").clicked() {
                                        let delete_id = app.theme.id.clone();
                                        app.custom_themes.retain(|t| t.id != delete_id);
                                        Database::save_custom_themes(&app.custom_themes);
                                        app.theme = ThemeConfig::cyber_cyan();
                                        Database::save_active_theme(&app.theme);
                                        app.set_toast("Custom Theme Deleted");
                                    }
                                }
                            });

                            ui.add_space(10.0);
                            ui.separator();
                            ui.add_space(10.0);

                            ui.label(egui::RichText::new("Theme Colors (Real-Time Customization)").strong().color(app.theme.accent_color()));
                            ui.label(egui::RichText::new("Click any color swatch below to open the color picker. Changes take effect instantly.").small().color(app.theme.text_muted_color()));
                            ui.add_space(8.0);

                            let mut color_modified = false;

                            egui::Grid::new("colors_grid").num_columns(4).spacing([18.0, 8.0]).show(ui, |ui| {
                                ui.label("Terminal Background:");
                                color_modified |= ui.color_edit_button_srgb(&mut app.theme.bg_main).changed();

                                ui.label("Panel & Nav Background:");
                                color_modified |= ui.color_edit_button_srgb(&mut app.theme.bg_panel).changed();
                                ui.end_row();

                                ui.label("Card Background:");
                                color_modified |= ui.color_edit_button_srgb(&mut app.theme.bg_card).changed();

                                ui.label("Borders & Dividers:");
                                color_modified |= ui.color_edit_button_srgb(&mut app.theme.border).changed();
                                ui.end_row();

                                ui.label("Primary Accent:");
                                color_modified |= ui.color_edit_button_srgb(&mut app.theme.accent).changed();

                                ui.label("Accent Hover:");
                                color_modified |= ui.color_edit_button_srgb(&mut app.theme.accent_hover).changed();
                                ui.end_row();

                                ui.label("Primary Text:");
                                color_modified |= ui.color_edit_button_srgb(&mut app.theme.text_primary).changed();

                                ui.label("Muted Text:");
                                color_modified |= ui.color_edit_button_srgb(&mut app.theme.text_muted).changed();
                                ui.end_row();

                                ui.label("Success Badge:");
                                color_modified |= ui.color_edit_button_srgb(&mut app.theme.success).changed();

                                ui.label("Danger / Close:");
                                color_modified |= ui.color_edit_button_srgb(&mut app.theme.danger).changed();
                                ui.end_row();

                                // Text rendered on top of accent-colored buttons
                                // and active tabs. [0,0,0] = auto (WCAG-based
                                // pick from the accent color). Click the swatch
                                // to set an explicit override.
                                ui.label("Text on Accent:");
                                let auto_pick = crate::modern::on_accent_text(app.theme.accent_color());
                                let is_auto = app.theme.on_accent == [0, 0, 0];
                                let mut display = if is_auto {
                                    [auto_pick.r(), auto_pick.g(), auto_pick.b()]
                                } else {
                                    app.theme.on_accent
                                };
                                if ui.color_edit_button_srgb(&mut display).changed() {
                                    app.theme.on_accent = display;
                                    color_modified = true;
                                }

                                // Reset-to-auto button (only enabled when
                                // an explicit override is set).
                                if !is_auto {
                                    if ui
                                        .small_button("auto")
                                        .on_hover_text("Reset to auto (based on accent)")
                                        .clicked()
                                    {
                                        app.theme.on_accent = [0, 0, 0];
                                        color_modified = true;
                                    }
                                } else {
                                    ui.label(
                                        egui::RichText::new("(auto)")
                                            .small()
                                            .color(app.theme.text_muted_color()),
                                    );
                                }
                                ui.end_row();
                            });

                            ui.add_space(10.0);
                            ui.label(egui::RichText::new("Terminal 16 ANSI Palette").strong().color(app.theme.text_primary_color()));
                            ui.add_space(6.0);

                            egui::Grid::new("ansi_grid").num_columns(8).spacing([10.0, 6.0]).show(ui, |ui| {
                                let labels = ["Black", "Red", "Green", "Yellow", "Blue", "Magenta", "Cyan", "White"];
                                for (i, name) in labels.iter().enumerate() {
                                    ui.vertical(|ui| {
                                        ui.label(egui::RichText::new(*name).small().color(app.theme.text_muted_color()));
                                        color_modified |= ui.color_edit_button_srgb(&mut app.theme.ansi_colors[i]).changed();
                                    });
                                }
                                ui.end_row();

                                let bright_labels = ["Br-Black", "Br-Red", "Br-Green", "Br-Yellow", "Br-Blue", "Br-Magenta", "Br-Cyan", "Br-White"];
                                for (i, name) in bright_labels.iter().enumerate() {
                                    ui.vertical(|ui| {
                                        ui.label(egui::RichText::new(*name).small().color(app.theme.text_muted_color()));
                                        color_modified |= ui.color_edit_button_srgb(&mut app.theme.ansi_colors[i + 8]).changed();
                                    });
                                }
                                ui.end_row();
                            });

                            if color_modified {
                                if !app.theme.is_builtin {
                                    if let Some(existing) = app.custom_themes.iter_mut().find(|t| t.id == app.theme.id) {
                                        *existing = app.theme.clone();
                                        Database::save_custom_themes(&app.custom_themes);
                                    }
                                }
                                Database::save_active_theme(&app.theme);
                            }
                        }
                        SettingsCategory::Terminal => {
                            ui.label(egui::RichText::new("Terminal Interaction").strong().size(16.0).color(app.theme.accent_color()));
                            ui.label(egui::RichText::new("Configure mouse behavior, clipboard actions, and scrollback depth.").small().color(app.theme.text_muted_color()));
                            ui.add_space(12.0);

                            changed |= setting_row_toggle(ui, "Cursor Blink", "Animate cursor blinking in the active terminal buffer.", &mut app.settings.cursor_blink, &app.theme);
                            changed |= setting_row_toggle(ui, "Copy Selected Text on Select", "Automatically copy highlighted text to OS clipboard on drag release.", &mut app.settings.copy_on_select, &app.theme);
                            changed |= setting_row_toggle(ui, "Paste on Right Click", "Immediately write clipboard text into the terminal on right click.", &mut app.settings.paste_on_right_click, &app.theme);

                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.label(egui::RichText::new("Mouse Wheel Scroll Amount").strong().color(app.theme.text_primary_color()));
                                    ui.label(egui::RichText::new("How far each wheel notch scrolls the terminal buffer. Page sizes adapt to window height.").small().color(app.theme.text_muted_color()));
                                });
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    egui::ComboBox::from_id_source("wheel_scroll_amount_combo")
                                        .selected_text(app.settings.mouse_wheel_scroll.display_name())
                                        .show_ui(ui, |ui| {
                                            use crate::settings::WheelScrollAmount::*;
                                            let opts = [Lines1, Lines3, Lines5, Lines10, HalfPage, FullPage];
                                            for opt in opts {
                                                if ui.selectable_value(&mut app.settings.mouse_wheel_scroll, opt, opt.display_name()).clicked() {
                                                    changed = true;
                                                }
                                            }
                                        });
                                });
                            });
                            ui.add_space(6.0);
                            ui.separator();
                            ui.add_space(6.0);

                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.label(egui::RichText::new("Scrollback Buffer Depth").strong().color(app.theme.text_primary_color()));
                                    ui.label(egui::RichText::new("Total lines of output history retained per tab (scroll with mouse wheel).").small().color(app.theme.text_muted_color()));
                                });
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    egui::ComboBox::from_id_source("scrollback_depth_combo")
                                        .selected_text(format!("{} lines", app.settings.scrollback_lines))
                                        .show_ui(ui, |ui| {
                                            if ui.selectable_value(&mut app.settings.scrollback_lines, 2000, "2,000 lines").clicked() { changed = true; }
                                            if ui.selectable_value(&mut app.settings.scrollback_lines, 5000, "5,000 lines").clicked() { changed = true; }
                                            if ui.selectable_value(&mut app.settings.scrollback_lines, 10000, "10,000 lines").clicked() { changed = true; }
                                            if ui.selectable_value(&mut app.settings.scrollback_lines, 25000, "25,000 lines").clicked() { changed = true; }
                                            if ui.selectable_value(&mut app.settings.scrollback_lines, 50000, "50,000 lines").clicked() { changed = true; }
                                        });
                                });
                            });
                            ui.add_space(6.0);
                            ui.separator();
                            ui.add_space(6.0);

                            setting_row_disabled(ui, "Right Click Auto Select Word", "Double click/right click to select full alphanumeric words.", app.settings.right_click_select_word);
                            setting_row_disabled(ui, "Hold Ctrl / Meta to Open Links", "Require modifier key press before launching detected URL hyperlinks.", app.settings.must_hold_ctrl_for_links);
                            setting_row_disabled(ui, "Command Suggestions", "Display autocompletion hints based on history.", app.settings.show_command_suggestions);
                            setting_row_disabled(ui, "Auto Reconnect on Disconnect", "Automatically retry remote SSH sessions when connection drops.", app.settings.auto_reconnect_terminal);
                        }
                        SettingsCategory::ShellEnv => {
                            ui.label(egui::RichText::new("Shell & Environment").strong().size(16.0).color(app.theme.accent_color()));
                            ui.label(egui::RichText::new("Set your default command interpreter, session log directories, and keycodes.").small().color(app.theme.text_muted_color()));
                            ui.add_space(12.0);

                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.label(egui::RichText::new("Default Shell Path").strong().color(app.theme.text_primary_color()));
                                    ui.label(egui::RichText::new("Executable path spawned when opening new local tabs.").small().color(app.theme.text_muted_color()));
                                });
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.add(egui::TextEdit::singleline(&mut app.settings.default_shell).desired_width(180.0)).changed() {
                                        changed = true;
                                    }
                                });
                            });
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("Shell Presets:").small().color(app.theme.text_muted_color()));
                                if crate::modern::button_small(ui, &app.theme, "bash").clicked() { app.settings.default_shell = "/bin/bash".to_string(); changed = true; }
                                if crate::modern::button_small(ui, &app.theme, "zsh").clicked() { app.settings.default_shell = "/bin/zsh".to_string(); changed = true; }
                                if crate::modern::button_small(ui, &app.theme, "fish").clicked() { app.settings.default_shell = "/bin/fish".to_string(); changed = true; }
                            });
                            ui.add_space(8.0);
                            ui.separator();
                            ui.add_space(8.0);

                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.label(egui::RichText::new("Backspace Keycode Sequence").strong().color(app.theme.text_primary_color()));
                                    ui.label(egui::RichText::new("Control character code sent to PTY upon pressing Backspace.").small().color(app.theme.text_muted_color()));
                                });
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    egui::ComboBox::from_id_source("backspace_seq_select")
                                        .selected_text(match app.settings.backspace_sequence {
                                            BackspaceSequence::Delete127 => "^? (Delete 0x7F)",
                                            BackspaceSequence::Backspace8 => "^H (Backspace 0x08)",
                                        })
                                        .show_ui(ui, |ui| {
                                            if ui.selectable_value(&mut app.settings.backspace_sequence, BackspaceSequence::Delete127, "^? (Delete 0x7F)").clicked() { changed = true; }
                                            if ui.selectable_value(&mut app.settings.backspace_sequence, BackspaceSequence::Backspace8, "^H (Backspace 0x08)").clicked() { changed = true; }
                                        });
                                });
                            });
                            ui.add_space(8.0);
                            ui.separator();
                            ui.add_space(8.0);

                            setting_row_disabled(ui, "Terminal Log Directory", "Target filesystem folder for saved session transcripts.", false);
                            setting_row_disabled(ui, "Save Terminal Session Logs", "Write all output streams into timestamped log files.", app.settings.save_terminal_log);
                            setting_row_disabled(ui, "Timestamp Log Entries", "Prefix each logged output line with local ISO timestamp.", app.settings.add_timestamp_to_log);
                        }
                        SettingsCategory::Sftp => {
                            ui.label(egui::RichText::new("SFTP & File Transfers").strong().size(16.0).color(app.theme.accent_color()));
                            ui.label(egui::RichText::new("Manage remote directory traversal, split paneling, and file syncing.").small().color(app.theme.text_muted_color()));
                            ui.add_space(12.0);

                            changed |= setting_row_toggle(ui, "Split View SFTP Explorer", "Show terminal on the left and directory browser on the right.", &mut app.settings.show_sftp_split_view, &app.theme);
                            changed |= setting_row_toggle(ui, "Synchronize SFTP with Terminal Path", "Automatically follow the current directory of the active shell.", &mut app.settings.sftp_path_sync, &app.theme);

                            setting_row_disabled(ui, "Auto Refresh on Tab Switch", "Query remote directory metadata when navigating between sessions.", app.settings.auto_refresh_sftp);
                            setting_row_disabled(ui, "Show Hidden Dotfiles", "Display files and folders prefixed with a dot by default.", app.settings.show_hidden_sftp);
                            setting_row_disabled(ui, "Disable SFTP Transfer History", "Do not write upload/download records to disk.", app.settings.disable_sftp_history);
                        }
                        SettingsCategory::System => {
                            ui.label(egui::RichText::new("Application & System").strong().size(16.0).color(app.theme.accent_color()));
                            ui.label(egui::RichText::new("Window behavior, multi-instance options, and update checks.").small().color(app.theme.text_muted_color()));
                            ui.add_space(12.0);

                            changed |= setting_row_toggle(ui, "Open Default Tab on Startup", "Spawn a fresh local shell if no previous session was restored.", &mut app.settings.open_default_tab, &app.theme);

                            if setting_row_toggle(ui, "Check for Updates on Startup", "Check for newer releases on GitHub.", &mut app.settings.check_updates, &app.theme) {
                                changed = true;
                                if !app.settings.check_updates {
                                    app.available_update = None;
                                    app.settings.pending_update = None;
                                    app.show_update_modal = false;
                                }
                            }

                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.label(egui::RichText::new("Manual Update Check").strong().color(app.theme.text_primary_color()));
                                    let status_text = if app.is_checking_update {
                                        "Checking GitHub releases...".to_string()
                                    } else if let Some(ref tag) = app.available_update {
                                        format!("New version available: {}", tag)
                                    } else {
                                        format!("Current version is v{}", env!("CARGO_PKG_VERSION"))
                                    };
                                    ui.label(egui::RichText::new(status_text).small().color(app.theme.text_muted_color()));
                                });
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if crate::modern::button_accent(ui, &app.theme, "Check for Updates Now").clicked() {
                                        app.trigger_update_check(true, ctx.clone());
                                        app.set_toast("Checking for updates...");
                                    }
                                });
                            });
                            ui.add_space(6.0);
                            ui.separator();
                            ui.add_space(6.0);

                            setting_row_disabled(ui, "Allow Multi-Instance Execution", "Permit launching multiple independent AZTerm window processes.", app.settings.allow_multi_instance);
                            setting_row_disabled(ui, "Confirm Before Window Exit", "Ask for confirmation before terminating running session processes.", app.settings.confirm_before_exit);
                            setting_row_disabled(ui, "Mask Host IP Address", "Hide server IPs from status bars and session titles.", app.settings.hide_ip);
                            setting_row_disabled(ui, "Disable Connection History", "Do not cache recent SSH session targets in SQLite.", app.settings.disable_connection_history);
                            changed |= setting_row_toggle(
                                ui,
                                "Debug Logging Mode",
                                "Write timestamped traces of PTY, SSH, SFTP and layout operations to a log file. Leave on if you are chasing a freeze or crash.",
                                &mut app.settings.debug_mode,
                                &app.theme,
                            );

                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.label(egui::RichText::new("Debug Log Path").strong().color(app.theme.text_primary_color()));
                                    ui.label(egui::RichText::new("File is appended to on each run; rotates to .old at 20 MB.").small().color(app.theme.text_muted_color()));
                                });
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.add(egui::TextEdit::singleline(&mut app.settings.debug_log_path).desired_width(280.0)).changed() {
                                        changed = true;
                                    }
                                });
                            });
                            ui.add_space(6.0);
                            ui.separator();
                            ui.add_space(6.0);
                        }
                    }
                });
            });

            if changed {
                app.settings.save();
            }
        });
    });
}
