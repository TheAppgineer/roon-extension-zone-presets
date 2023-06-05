use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use serde_repr::{Deserialize_repr, Serialize_repr};
use rust_roon_api::{RoonApi, CoreEvent, Info, Parsed, RespProps, Services, Svc, send_complete, send_continue_all, info};
use rust_roon_api::status::{self, Status};
use rust_roon_api::settings::{self, Settings, Widget, Dropdown, Group, Label, Layout, Textbox};
use rust_roon_api::transport::{Transport, Output, Zone};

#[derive(Clone, Debug, Default, Deserialize_repr, Serialize_repr)]
#[repr(usize)]
#[serde(rename_all = "snake_case")]
enum Action {
    #[default] Edit = 0,
    Activate = 1,
    Deactivate = 2,
    Delete = 3
}

#[derive(Clone, Debug, Default, Deserialize_repr, Serialize_repr)]
#[repr(usize)]
#[serde(rename_all = "snake_case")]
enum VolumeType {
    Current = 1,
    #[default] Preset = 2
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct Preset {
    name: String,
    output_ids: Vec<String>,
    volume_type: VolumeType,
    volumes: HashMap<String, i32>
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct GroupingSettings {
    selected: Option<usize>,
    action: Action,
    add: Option<String>,
    primary_output_id: Option<String>,
    name: String,
    output_ids: Vec<String>,
    volume_type: VolumeType,
    volumes: HashMap<String, i32>,
    presets: Vec<Preset>,
    extracted_preset: Option<Preset>
}

fn store_preset(settings: &mut GroupingSettings) -> Option<()> {
    let name = settings.name.to_owned();
    let add = settings.add.to_owned()?;
    let primary_output_id = settings.primary_output_id.to_owned()?;
    let mut output_ids = settings.output_ids.to_owned();

    if output_ids.len() == 0 {
        output_ids.push(primary_output_id.to_owned());
        settings.output_ids.push(primary_output_id);
    }

    if !output_ids.contains(&add) {
        output_ids.push(add.to_owned());
        settings.output_ids.push(add);
    }

    if name.len() > 0 && output_ids.len() > 0 {
        let preset = Preset {
            name,
            output_ids,
            ..Default::default()
        };

        if let Some(selected) = settings.selected {
            let preset_count = settings.presets.len();

            if selected < preset_count {
                settings.presets[selected] = preset;
            } else {
                settings.selected = Some(preset_count);
                settings.presets.push(preset);
            }
        }

        Some(())
    } else {
        None
    }
}

fn load_preset(settings: &mut GroupingSettings) {
    if let Some(selected) = settings.selected {
        if let Some(preset) = settings.extracted_preset.as_ref() {
            settings.name = preset.name.to_owned();
            settings.primary_output_id = Some(preset.output_ids[0].to_owned());
            settings.output_ids = preset.output_ids.to_owned();
            settings.action = Action::Edit;
            settings.add = settings.output_ids.get(0).cloned();
        } else {
            if let Some(preset) = settings.presets.get(selected) {
                settings.name = preset.name.to_owned();
                settings.primary_output_id = Some(preset.output_ids[0].to_owned());
                settings.output_ids = preset.output_ids.to_owned();
            } else {
                settings.name = String::new();
                settings.primary_output_id = None;
                settings.output_ids = Vec::new();
                settings.action = Action::Edit;
            }

            settings.add = None;
        }
    }
}

fn match_preset<'a, 'b>(presets: &'a Vec<Preset>, zones: &'b Vec<Zone>) -> Option<(&'a Preset, &'b Zone)> {
    for preset in presets {
        for zone in zones {
            if zone.outputs.len() == preset.output_ids.len() {
                let output_ids: Vec<&str> = zone.outputs
                    .iter()
                    .map(|output| output.output_id.as_str())
                    .collect();
                let match_count = preset.output_ids.iter().zip(output_ids).filter(|(a, b)| a == b).count();

                if match_count == preset.output_ids.len() {
                    return Some((preset, zone))
                }
            }
        }
    }

    None
}

fn extract_preset(zones: &Vec<Zone>) -> Option<Preset> {
    for zone in zones {
        if zone.outputs.len() > 1 {
            let mut preset = Preset::default();

            preset.name = zone.display_name.to_owned();

            for output in &zone.outputs {
                preset.output_ids.push(output.output_id.to_owned());
            }

            return Some(preset)
        }
    }

    None
}

fn make_layout(settings: GroupingSettings, outputs: &HashMap<String, Output>) -> Layout<GroupingSettings> {
    let has_error = false;
    let is_selected = settings.selected.is_some();
    let mut widgets = Vec::new();
    let mut preset_list = vec![HashMap::from([ ("title", "(select preset)".into()), ("value", Value::Null) ])];

    for index in 0..settings.presets.len() {
        let name = settings.presets[index].name.to_owned();

        if name.len() > 0 {
            preset_list.push(HashMap::from([ ("title", name.into()), ("value", index.into()) ]));
        }
    }

    preset_list.push(HashMap::from([ ("title", "New Preset".into()), ("value", settings.presets.len().into()) ]));

    let selected = Widget::Dropdown(Dropdown {
        title: "Preset",
        subtitle: None,
        values: preset_list,
        setting: "selected"
    });

    widgets.push(selected);

    if is_selected {
        let is_new_preset = settings.selected.unwrap() == settings.presets.len();

        if !is_new_preset {
            let mut actions = Vec::new();

            actions.push(HashMap::from([ ("title", "(select action)".into()), ("value", Value::Null) ]));
            actions.push(HashMap::from([ ("title", "Activate".into()), ("value", (Action::Activate as usize).into()) ]));
            actions.push(HashMap::from([ ("title", "Deactivate".into()), ("value", (Action::Deactivate as usize).into()) ]));
            actions.push(HashMap::from([ ("title", "Edit".into()), ("value", (Action::Edit as usize).into()) ]));
            actions.push(HashMap::from([ ("title", "Delete".into()), ("value", (Action::Delete as usize).into()) ]));

            let action = Widget::Dropdown(Dropdown {
                title: "Action",
                subtitle: None,
                values: actions,
                setting: "action"
            });

            widgets.push(action);
        }

        match settings.action {
            Action::Edit => {
                let name = Widget::Textbox(Textbox {
                    title: "Name",
                    subtitle: None,
                    setting: "name"
                });
                let mut edit_group = Widget::Group(Group {
                    title: "Preset Editor",
                    subtitle: None,
                    collapsable: true,
                    items: vec![name]
                });
    
                if settings.name.len() > 0 {
                    if let Widget::Group(edit_group) = &mut edit_group {
                        let mut values = vec![HashMap::from(
                            [ ("title", "(select output)".into()), ("value", Value::Null) ]
                        )];
    
                        for (output_id, output) in outputs {
                            values.push(HashMap::from(
                                [ ("title", output.display_name.to_owned().into()), ("value", output_id.to_owned().into()) ]
                            ));
                        }
    
                        let output = Widget::Dropdown(Dropdown {
                            title: "Primary Output",
                            subtitle: None,
                            values,
                            setting: "primary_output_id"
                        });
    
                        edit_group.items.push(output);
    
                        if let Some(primary_output_id) = &settings.primary_output_id {
                            if let Some(output) = outputs.get(primary_output_id) {
                                let mut values = vec![HashMap::from([ ("title", "(select output)".into()), ("value", Value::Null) ])];
    
                                for output_id in &output.can_group_with_output_ids {
                                    if *output_id != *primary_output_id {
                                        let name = outputs.get(output_id).unwrap().display_name.to_owned();
    
                                        values.push(HashMap::from([ ("title", name.into()), ("value", output_id.to_owned().into()) ]));
                                    }
                                }
    
                                edit_group.items.push(Widget::Dropdown(Dropdown {
                                    title: "Group With",
                                    subtitle: None,
                                    values,
                                    setting: "add"
                                }));
                            }
                        }
                    }
                }
    
                widgets.push(edit_group);
            }
            _ => ()
        }

        if let Some(primary_output_id) = &settings.primary_output_id {
            let name = outputs.get(primary_output_id).unwrap().display_name.to_owned();
            let mut subtitle = String::from("Grouped with:");

            for output_id in &settings.output_ids {
                if output_id == primary_output_id {
                    continue;
                }

                if let Some(sec_output) = outputs.get(output_id) {
                    subtitle.push('\n');
                    subtitle.push_str(&sec_output.display_name);
                }
            }

            widgets.push(Widget::Label(Label {
                title: name.to_owned(),
                subtitle: Some(subtitle)
            }));
        }
    }

    Layout {
        settings,
        widgets,
        has_error
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut roon = RoonApi::new(info!("com.theappgineer", "Zone Presets"));
    let mut provided: HashMap<String, Svc> = HashMap::new();
    let output_list = Arc::new(Mutex::new(HashMap::new()));
    let last_selected = Arc::new(Mutex::new(None));
    let settings = serde_json::from_value::<GroupingSettings>(RoonApi::load_config("settings")).unwrap_or_default();
    let grouping_settings = Arc::new(Mutex::new(settings));

    let output_list_clone = output_list.clone();
    let last_selected_clone = last_selected.clone();
    let grouping_settings_clone = grouping_settings.clone();
    let get_settings_cb = move |cb: fn(Layout<GroupingSettings>) -> Vec<RespProps>| -> Vec<RespProps> {
        let output_list = output_list_clone.lock().unwrap();
        let mut last_selected = last_selected_clone.lock().unwrap();
        let settings = grouping_settings_clone.lock().unwrap();

        *last_selected = settings.selected;

        cb(make_layout(settings.to_owned(), &output_list))
    };

    let output_list_clone = output_list.clone();
    let save_settings_cb = move |is_dry_run: bool, mut settings: GroupingSettings| -> Vec<RespProps> {
        let output_list = output_list_clone.lock().unwrap();
        let mut last_selected = last_selected.lock().unwrap();
        let mut resp_props: Vec<RespProps> = Vec::new();

        if let Action::Delete = settings.action {
            if let Some(index) = settings.selected {
                if index < settings.presets.len() {
                    settings.presets.remove(index);
                    settings.selected = None;
                }
            }
        }

        if settings.selected != *last_selected {
            load_preset(&mut settings);

            *last_selected = settings.selected;
        } else {
            store_preset(&mut settings);
        }

        let layout = make_layout(settings, &output_list);
        let layout = layout.serialize(serde_json::value::Serializer).unwrap();

        send_complete!(resp_props, "Success", Some(json!({"settings": layout})));

        if !is_dry_run && !layout["has_error"].as_bool().unwrap() {
            send_continue_all!(resp_props, "subscribe_settings", "Changed", Some(json!({"settings": layout})));
        }

        resp_props
    };
    let (svc, settings) = Settings::new(&roon, Box::new(get_settings_cb), Box::new(save_settings_cb));

    provided.insert(settings::SVCNAME.to_owned(), svc);

    let (svc, status) = Status::new(&roon);

    provided.insert(status::SVCNAME.to_owned(), svc);

    let services = vec![
        Services::Settings(settings),
        Services::Status(status),
        Services::Transport(Transport::new())
    ];
    let (mut handles, mut core_rx) = roon.start_discovery(provided, Some(services)).await.unwrap();

    let core_handler = async move {
        let mut status = None;
        let mut transport = None;
        let mut matched_zone_id = None;

        loop {
            if let Some((core, msg)) = core_rx.recv().await {
                match core {
                    CoreEvent::Found(mut core) => {
                        println!("Core found: {}, version {}", core.display_name, core.display_version);

                        status = core.get_status().cloned();

                        if let Some(status) = status.as_ref() {
                            status.set_status("No preset active".to_owned(), false).await;
                        };

                        transport = core.get_transport().cloned();

                        if let Some(transport) = transport.as_ref() {
                            transport.subscribe_zones().await;
                            transport.subscribe_outputs().await;
                        }
                    }
                    CoreEvent::Lost(core) => {
                        println!("Core lost: {}, version {}", core.display_name, core.display_version);
                    }
                    _ => ()
                }

                if let Some((_, parsed)) = msg {
                    match parsed {
                        Parsed::Zones(zones) => {
                            if matched_zone_id.is_none() {
                                let mut presets = grouping_settings.lock().unwrap().presets.to_owned();

                                if let Some((matching_preset, zone)) = match_preset(&mut presets, &zones) {
                                    let status_msg = format!(
                                        "Grouped zone \"{}\" represents the \"{}\" preset", 
                                        zone.display_name,
                                        matching_preset.name
                                    );

                                    matched_zone_id = Some(zone.zone_id.to_owned());

                                    if let Some(status) = status.as_ref() {
                                        status.set_status(status_msg, false).await;
                                    }
                                }
                            }

                            let mut settings = grouping_settings.lock().unwrap();

                            settings.extracted_preset = extract_preset(&zones);
                        }
                        Parsed::ZonesRemoved(removed_zone_ids) => {
                            if let Some(zone_id) = &matched_zone_id {
                                if removed_zone_ids.contains(zone_id) {
                                    matched_zone_id = None;

                                    if let Some(status) = status.as_ref() {
                                        status.set_status("No preset active".to_owned(), false).await;
                                    }
                                }
                            }
                        }
                        Parsed::Outputs(outputs) => {
                            for output in outputs {
                                let output_id = output.output_id.to_owned();
                                let mut output_list = output_list.lock().unwrap();

                                output_list.insert(output_id, output);
                            }
                        }
                        Parsed::SettingsSaved(settings) => {
                            let mut nv_settings = settings.to_owned();

                            nv_settings["extracted_preset"] = serde_json::Value::Null;

                            RoonApi::save_config("settings", nv_settings).unwrap();

                            if let Ok(settings) = serde_json::from_value::<GroupingSettings>(settings) {
                                let mut status_msg = "Settings saved".to_owned();

                                if settings.selected.is_some() && settings.primary_output_id.is_some() {
                                    if let Some(transport) = transport.as_ref() {
                                        let output_ids = settings.output_ids
                                            .iter()
                                            .map(|value| value.as_str())
                                            .collect();

                                        match settings.action {
                                            Action::Activate => {
                                                transport.group_outputs(output_ids).await;
                                                status_msg = format!("Preset \"{}\" activated", settings.name);
                                            }
                                            Action::Deactivate => {
                                                transport.ungroup_outputs(output_ids).await;
                                                status_msg = format!("Preset \"{}\" deactivated", settings.name);
                                            }
                                            Action::Edit => {
                                                transport.get_zones().await;
                                            }
                                            _ => ()
                                        }
                                    }
                                }

                                if let Action::Delete = settings.action {
                                    matched_zone_id = None;
                                    status_msg = format!("Preset \"{}\" deleted", settings.name);
                                }

                                if let Some(status) = status.as_ref() {
                                    status.set_status(status_msg, false).await;
                                }

                                let mut grouping_settings = grouping_settings.lock().unwrap();

                                if *grouping_settings.name != settings.name {
                                    // A name change requires new matching
                                    matched_zone_id = None;
                                }

                                *grouping_settings = settings;
                            }
                        }
                        _ => ()
                    }
                }
            }
        }
    };

    handles.push(tokio::spawn(core_handler));

    for handle in handles {
        handle.await.unwrap();
    }
}