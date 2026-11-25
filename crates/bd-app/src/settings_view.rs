//! Settings screen: builds the visible options from the schema and applies
//! changes.

use bd_core::modules::ModuleStatus;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::app::App;
use crate::schema::{self, Kind, Status, CATEGORIES, DEFS, MODULES_CATEGORY};
use crate::{i18n, platform, wiring, AppWindow, GrabberState, SettingItem, SettingsState};

fn matches(query: &str, parts: &[&str]) -> bool {
    query.is_empty() || parts.iter().any(|p| p.to_lowercase().contains(query))
}

fn options_of(kind: Kind) -> (Vec<SharedString>, Vec<&'static str>) {
    match kind {
        Kind::Select { options, .. } => (
            options.iter().map(|(_, en, fr)| SharedString::from(if i18n::is_french() { *fr } else { *en })).collect(),
            options.iter().map(|(v, _, _)| *v).collect(),
        ),
        _ => (Vec::new(), Vec::new()),
    }
}

pub fn refresh(app: &App, ui: &AppWindow) {
    let s = ui.global::<SettingsState>();
    let categories: Vec<SharedString> =
        (0..CATEGORIES.len()).map(|i| SharedString::from(schema::category_name(i))).collect();
    s.set_categories(ModelRc::new(VecModel::from(categories)));
    s.set_config_path(app.settings_path.as_ref().map(|p| p.display().to_string()).unwrap_or_default().into());

    let query = s.get_search().to_lowercase();
    let query = query.trim().to_owned();
    let category = s.get_category().max(0) as usize;
    let show_advanced = s.get_show_advanced() || !query.is_empty();
    let mut items = Vec::new();
    let mut hidden = 0;
    let mut last_group = usize::MAX;

    for cat in 0..CATEGORIES.len() {
        if query.is_empty() && cat != category {
            continue;
        }
        let group = schema::category_name(cat);
        if cat == MODULES_CATEGORY {
            for (info, enabled) in app.modules.list() {
                if !matches(&query, &[info.name, info.description, group, info.category.label()]) {
                    continue;
                }
                items.push(SettingItem {
                    id: format!("module:{}", info.id).into(),
                    label: format!("{} · {}", info.category.label(), info.name).into(),
                    hint: info.description.into(),
                    kind: 5,
                    flag: enabled,
                    status: if info.status == ModuleStatus::Available { 0 } else { 2 },
                    group: group.into(),
                    first_in_group: last_group != cat,
                    ..Default::default()
                });
                last_group = cat;
            }
            continue;
        }
        for def in DEFS.iter().filter(|d| d.cat == cat) {
            if !matches(&query, &[def.label(), def.hint(), def.en, group]) {
                continue;
            }
            if def.advanced && !show_advanced {
                hidden += 1;
                continue;
            }
            let value = schema::get(&app.settings, def.id);
            let (options, values) = options_of(def.kind);
            let (kind, min, max, unit) = match def.kind {
                Kind::Toggle(_) => (0, 0, 0, ""),
                Kind::Number { min, max, unit, .. } => (1, min, max, unit),
                Kind::Select { .. } => (2, 0, 0, ""),
                Kind::Text(_) => (3, 0, 0, ""),
                Kind::Path => (4, 0, 0, ""),
                Kind::Button(..) => (6, 0, 0, ""),
            };
            items.push(SettingItem {
                id: def.id.into(),
                label: def.label().into(),
                hint: def.hint().into(),
                info: def.info().into(),
                kind,
                flag: value == "true",
                number: value.parse::<i64>().unwrap_or(0).clamp(i32::MIN as i64, i32::MAX as i64) as i32,
                text: match def.kind {
                    Kind::Button(en, fr) => if i18n::is_french() { fr } else { en }.into(),
                    _ => value.as_str().into(),
                },
                option_index: values.iter().position(|v| *v == value).unwrap_or(0) as i32,
                options: ModelRc::new(VecModel::from(options)),
                unit: unit.into(),
                min: min.clamp(i32::MIN as i64, i32::MAX as i64) as i32,
                max: max.clamp(i32::MIN as i64, i32::MAX as i64) as i32,
                status: match def.status {
                    Status::Ready => 0,
                    Status::Restart => 1,
                },
                advanced: def.advanced,
                group: group.into(),
                first_in_group: last_group != cat,
            });
            last_group = cat;
        }
    }
    s.set_items(ModelRc::new(VecModel::from(items)));
    s.set_rules(ModelRc::new(VecModel::from(crate::scheduler::rule_rows(&app.settings))));
    s.set_hidden_advanced(if query.is_empty() { hidden } else { 0 });
}

/// Applies a change coming from the UI. `value` is "true"/"false" for
/// toggles, a number, the option index for selects, or text.
pub fn changed(app: &mut App, ui: &AppWindow, id: &str, value: &str) {
    if let Some(module) = id.strip_prefix("module:") {
        wiring::set_module(app, ui, module, value == "true");
        refresh(app, ui);
        return;
    }
    let Some(def) = schema::def(id) else { return };
    if let Kind::Button(..) = def.kind {
        crate::actions::button(app, ui, id);
        return;
    }
    let stored = match def.kind {
        Kind::Select { options, .. } => {
            let index = value.parse::<usize>().unwrap_or(0).min(options.len() - 1);
            options[index].0.to_owned()
        }
        _ => value.to_owned(),
    };
    schema::set(&mut app.settings, id, &stored);
    match id {
        "theme" => ui.invoke_apply_theme(match stored.as_str() {
            "light" => 1,
            "dark" => 2,
            _ => 0,
        }),
        "language" => {
            i18n::select(i18n::index_of(&stored));
            app.refresh_accounts(ui);
            app.refresh_grabber(ui, true);
            app.rebuild_tray();
        }
        "auto-start-grabbed" => ui.global::<GrabberState>().set_auto_start(app.settings.auto_start_grabbed),
        "clicknload-port" => app.restart_clicknload(),
        "launch-with-windows" => platform::set_launch_with_windows(stored == "true"),
        "density" => ui.set_compact(stored == "compact"),
        "portable" => app.toast(crate::i18n::t("Applies at the next start")),
        _ => {}
    }
    app.apply_config();
    app.save_settings();
    // Only rebuild the list when the shape changes, not on every keystroke.
    if matches!(def.kind, Kind::Select { .. } | Kind::Toggle(_)) || id == "language" {
        refresh(app, ui);
    }
}

pub fn reset_page(app: &mut App, ui: &AppWindow) {
    let category = ui.global::<SettingsState>().get_category().max(0) as usize;
    schema::reset_category(&mut app.settings, category);
    app.apply_config();
    app.save_settings();
    refresh(app, ui);
    app.toast(format!("{} ↺", schema::category_name(category)));
}

pub fn copy_json(app: &mut App) {
    match serde_json::to_string_pretty(&app.settings) {
        Ok(json) => {
            if platform::copy_text(&json) {
                app.ignore_clipboard = Some(json);
                app.toast("JSON ✓");
            }
        }
        Err(e) => app.toast(e.to_string()),
    }
}

pub fn browse(app: &mut App, ui: &AppWindow, id: &str) {
    let start = app.settings.download_dir.clone();
    if let Some(folder) = rfd::FileDialog::new().set_directory(start).pick_folder() {
        changed(app, ui, id, &folder.display().to_string());
        refresh(app, ui);
    }
}
