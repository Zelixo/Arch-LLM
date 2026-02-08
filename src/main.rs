use gtk4 as gtk;
use gtk::glib;
use gtk::prelude::*;
use std::rc::Rc;
use std::cell::RefCell;
use gtk::{
    Application, ApplicationWindow, Box, Orientation, Label, Entry, Button,
    ScrolledWindow, ListBox, DropDown, StringList, Stack, StackSidebar,
    Popover, GestureClick, EventControllerKey, Spinner, TextView
};
use std::sync::{Arc, Mutex};
use serde_json;
use std::fs;
use ollama_rs::generation::chat::request::ChatMessageRequest;
use ollama_rs::generation::chat::ChatMessage;
use ollama_rs::Ollama;
use futures_util::StreamExt;
use directories::ProjectDirs;
use std::path::PathBuf;

mod state;
mod utils;

use state::{AppState, Agent, Profile, Settings, ChatHistory, ChatEvent};
use utils::{normalize_url, parse_markdown, markdown_to_pango, MarkdownBlock};

fn get_config_files() -> (PathBuf, PathBuf, PathBuf) {
    let dirs = ProjectDirs::from("org", "archllm", "arch-llm").expect("Could not determine project directories");
    
    let config_dir = dirs.config_dir();
    let data_dir = dirs.data_dir();
    let memory_dir = data_dir.join("memories");

    fs::create_dir_all(config_dir).expect("Could not create config directory");
    fs::create_dir_all(data_dir).expect("Could not create data directory");
    fs::create_dir_all(&memory_dir).expect("Could not create memory directory");

    (
        config_dir.join("settings.json"),
        data_dir.join("history.json"),
        memory_dir
    )
}

#[tokio::main]
async fn main() -> glib::ExitCode {
    println!("Arch-LLM v0.2 Started");
    let app = Application::builder()
        .application_id("org.archllm.ollama_chat")
        .build();

    app.connect_activate(build_ui);
    app.run()
}

fn build_ui(app: &Application) {
    let (settings_path, history_path, memory_path) = get_config_files();

    let history_data = fs::read_to_string(&history_path)
        .ok()
        .and_then(|s| serde_json::from_str::<Vec<ChatHistory>>(&s).ok())
        .unwrap_or_default();

    let mut settings_data = fs::read_to_string(&settings_path)
        .ok()
        .and_then(|s| serde_json::from_str::<Settings>(&s).ok())
        .unwrap_or_else(|| Settings::default());

    // Ensure all profiles have IDs
    let mut modified = false;
    for profile in &mut settings_data.profiles {
        if profile.id.is_empty() {
            profile.id = glib::uuid_string_random().to_string();
            modified = true;
        }
    }
    if modified {
        let _ = fs::write(&settings_path, serde_json::to_string(&settings_data).unwrap());
    }

    let ollama_url = normalize_url(&settings_data.ollama_endpoint);
    let ollama = Ollama::from_url(
        url::Url::parse(&ollama_url).unwrap_or_else(|_| url::Url::parse("http://localhost:11434").unwrap())
    );

    let state = Arc::new(Mutex::new(AppState {
        ollama,
        current_agent_idx: 0,
        messages: Vec::new(),
        history: history_data,
        settings: settings_data.clone(),
        config_path: settings_path,
        history_path,
        memory_path,
        current_task: None,
        available_models: Vec::new(),
    }));

    // --- Root Stack (Loading -> Error -> Main) ---
    let root_stack = Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .build();

    // Loading Page
    let loading_box = Box::builder()
        .orientation(Orientation::Vertical)
        .valign(gtk::Align::Center)
        .halign(gtk::Align::Center)
        .spacing(20)
        .build();
    let loading_spinner = Spinner::builder().spinning(true).build();
    loading_spinner.set_size_request(64, 64);
    loading_box.append(&loading_spinner);
    loading_box.append(&Label::new(Some("Connecting to Ollama...")));
    root_stack.add_named(&loading_box, Some("loading"));

    // Setup / Error Page
    let error_box = Box::builder()
        .orientation(Orientation::Vertical)
        .valign(gtk::Align::Center)
        .halign(gtk::Align::Center)
        .spacing(20)
        .width_request(400)
        .build();
    let error_icon = Label::builder().label("🌐").css_classes(["welcome-icon"]).build();
    let error_label = Label::builder()
        .label("Could not connect to Ollama.\nPlease check your endpoint address.")
        .justify(gtk::Justification::Center)
        .build();
    
    let endpoint_entry_setup = Entry::builder()
        .placeholder_text("http://localhost:11434")
        .text(&settings_data.ollama_endpoint)
        .build();
    
    let retry_btn = Button::with_label("Connect");
    retry_btn.add_css_class("suggested-action");
    
    error_box.append(&error_icon);
    error_box.append(&error_label);
    error_box.append(&endpoint_entry_setup);
    error_box.append(&retry_btn);
    root_stack.add_named(&error_box, Some("error"));

    let main_stack = Stack::builder()
        .transition_type(gtk::StackTransitionType::SlideLeftRight)
        .build();
    
    root_stack.add_named(&main_stack, Some("main"));

    let chat_box_container = Box::builder()
        .orientation(Orientation::Horizontal)
        .build();

    // --- Sidebar ---
    let sidebar = Box::builder()
        .orientation(Orientation::Vertical)
        .css_name("sidebar")
        .build();
    
    let sidebar_top = Box::builder()
        .orientation(Orientation::Vertical)
        .vexpand(true)
        .build();
    sidebar.append(&sidebar_top);

    let new_chat_btn = Button::with_label("New chat");
    new_chat_btn.set_margin_start(10);
    new_chat_btn.set_margin_end(10);
    new_chat_btn.set_margin_top(10);
    new_chat_btn.set_margin_bottom(10);
    sidebar_top.append(&new_chat_btn);
    
    let history_list = ListBox::builder()
        .margin_top(20)
        .css_classes(["history-list"])
        .build();
    let history_scrolled = ScrolledWindow::builder()
        .child(&history_list)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build();
    sidebar_top.append(&history_scrolled);

    let settings_btn = Button::with_label("Settings");
    settings_btn.set_margin_start(10);
    settings_btn.set_margin_end(10);
    settings_btn.set_margin_bottom(20);
    sidebar.append(&settings_btn);

    // --- Main Content Area ---
    let content_area = Box::builder()
        .orientation(Orientation::Vertical)
        .hexpand(true)
        .build();

    let header = Box::builder()
        .orientation(Orientation::Horizontal)
        .margin_start(20)
        .margin_end(20)
        .margin_top(20)
        .margin_bottom(20)
        .build();
    
    let agent_names_list = StringList::new(&[]);
    let agent_dropdown = DropDown::builder()
        .model(&agent_names_list)
        .build();
    header.append(&agent_dropdown);

    let refresh_agent_dropdown_func = |state: Arc<Mutex<AppState>>, agent_names_list: StringList| {
        let names: Vec<String> = {
            let s = state.lock().expect("Failed to lock state for agent dropdown refresh");
            s.settings.agents.iter().map(|a| a.name.clone()).collect()
        };
        let name_refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        agent_names_list.splice(0, agent_names_list.n_items(), &name_refs);
    };

    refresh_agent_dropdown_func(state.clone(), agent_names_list.clone());
    content_area.append(&header);

    // Chat display
    let scrolled_window = ScrolledWindow::builder()
        .vexpand(true)
        .build();
    let chat_box = Box::builder()
        .orientation(Orientation::Vertical)
        .spacing(10)
        .margin_start(100)
        .margin_end(100)
        .margin_top(20)
        .margin_bottom(20)
        .build();
    scrolled_window.set_child(Some(&chat_box));
    content_area.append(&scrolled_window);

    let scroll_to_bottom = {
        let scrolled_window = scrolled_window.clone();
        move || {
            let vadj = scrolled_window.vadjustment();
            vadj.set_value(vadj.upper() - vadj.page_size());
        }
    };

    let render_chat = {
        let chat_box = chat_box.clone();
        let scroll_to_bottom = scroll_to_bottom.clone();
        move |messages: &Vec<ChatMessage>| {
            while let Some(child) = chat_box.first_child() {
                chat_box.remove(&child);
            }
            
            if messages.is_empty() {
                let welcome = Box::builder()
                    .orientation(Orientation::Vertical)
                    .valign(gtk::Align::Center)
                    .halign(gtk::Align::Center)
                    .spacing(20)
                    .margin_top(50)
                    .build();
                let icon = Label::builder().label("🤖").css_classes(["welcome-icon"]).build();
                let text = Label::builder().label("Select an agent or start typing...").css_classes(["welcome-text"]).build();
                welcome.append(&icon);
                welcome.append(&text);
                chat_box.append(&welcome);
            } else {
                for msg in messages {
                    if msg.role == ollama_rs::generation::chat::MessageRole::System { continue; }
                    let is_user = msg.role == ollama_rs::generation::chat::MessageRole::User;
                    
                    let msg_container = Box::builder()
                        .orientation(Orientation::Vertical)
                        .spacing(5)
                        .margin_bottom(10)
                        .build();
                    
                    if is_user {
                        msg_container.set_halign(gtk::Align::End);
                    } else {
                        msg_container.set_halign(gtk::Align::Start);
                        let header_box = Box::builder().orientation(Orientation::Horizontal).spacing(10).build();
                        let header = Label::builder()
                            .label("Ollama")
                            .css_classes(["msg-header"])
                            .halign(gtk::Align::Start)
                            .hexpand(true)
                            .build();
                        header_box.append(&header);
                        
                        let copy_btn = Button::builder()
                            .icon_name("edit-copy-symbolic")
                            .css_classes(["flat"])
                            .valign(gtk::Align::Center)
                            .tooltip_text("Copy Response")
                            .build();
                        
                        let content = msg.content.clone();
                        copy_btn.connect_clicked(move |_| {
                            if let Some(display) = gtk::gdk::Display::default() {
                                display.clipboard().set(&content);
                            }
                        });
                        header_box.append(&copy_btn);
                        
                        msg_container.append(&header_box);
                    }

                    let blocks = parse_markdown(&msg.content);
                    for block in blocks {
                        match block {
                            MarkdownBlock::Text(text) => {
                                let label = Label::builder()
                                    .xalign(0.0)
                                    .wrap(true)
                                    .css_classes([if is_user { "user-message" } else { "bot-message" }])
                                    .build();
                                label.set_markup(&text);
                                if is_user {
                                    label.set_halign(gtk::Align::End);
                                } else {
                                    label.set_halign(gtk::Align::Start);
                                }
                                msg_container.append(&label);
                            }
                            MarkdownBlock::Code(_lang, code) => {
                                let buffer = gtk::TextBuffer::builder().text(&code).build();
                                let view = gtk::TextView::builder()
                                    .buffer(&buffer)
                                    .editable(false)
                                    .monospace(true)
                                    .wrap_mode(gtk::WrapMode::WordChar)
                                    .bottom_margin(10)
                                    .top_margin(10)
                                    .left_margin(10)
                                    .right_margin(10)
                                    .css_classes(["code-view"])
                                    .build();
                                
                                let frame = gtk::Frame::builder()
                                    .child(&view)
                                    .css_classes(["code-frame"])
                                    .build();
                                msg_container.append(&frame);
                            }
                        }
                    }
                    chat_box.append(&msg_container);
                }
                scroll_to_bottom();
            }
        }
    };

    render_chat(&state.lock().unwrap().messages);

    // Input area
    let input_container = Box::builder()
        .orientation(Orientation::Vertical)
        .margin_bottom(30)
        .margin_start(100)
        .margin_end(100)
        .build();

    let input_box = Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .build();

    let input_scroll = ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .min_content_height(50)
        .max_content_height(150)
        .hexpand(true)
        .build();

    let text_view = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .hexpand(true)
        .css_classes(["chat-input"])
        .build();
    input_scroll.set_child(Some(&text_view));

    let send_btn = Button::with_label("Send");
    send_btn.set_valign(gtk::Align::End);
    send_btn.add_css_class("send-btn");

    input_box.append(&input_scroll);
    input_box.append(&send_btn);
    input_container.append(&input_box);
    content_area.append(&input_container);

    chat_box_container.append(&sidebar);
    chat_box_container.append(&content_area);

    // --- Settings View ---
    let settings_view = Box::builder()
        .orientation(Orientation::Vertical)
        .build();

    let settings_header = Box::builder()
        .orientation(Orientation::Horizontal)
        .margin_start(20)
        .margin_end(20)
        .margin_top(20)
        .build();

    let back_btn = Button::with_label("← Back to Chat");
    settings_header.append(&back_btn);
    settings_view.append(&settings_header);

    let settings_content = Box::builder()
        .orientation(Orientation::Horizontal)
        .vexpand(true)
        .build();
    settings_view.append(&settings_content);

    let settings_stack = Stack::builder()
        .transition_type(gtk::StackTransitionType::SlideLeftRight)
        .hexpand(true)
        .build();
    
    let settings_stack_sidebar = StackSidebar::builder()
        .stack(&settings_stack)
        .build();

    settings_content.append(&settings_stack_sidebar);
    settings_content.append(&settings_stack);

    // --- General Settings ---
    let general_box = Box::builder()
        .orientation(Orientation::Vertical)
        .margin_start(20)
        .margin_end(20)
        .margin_top(20)
        .spacing(10)
        .build();
    
    general_box.append(&Label::new(Some("Ollama Endpoint")));
    let endpoint_entry = Entry::builder()
        .text(&state.lock().unwrap().settings.ollama_endpoint)
        .build();
    general_box.append(&endpoint_entry);

    let save_btn = Button::with_label("Save Settings");
    let state_save = state.clone();
    let endpoint_entry_clone = endpoint_entry.clone();
    save_btn.connect_clicked(move |_| {
        let endpoint = endpoint_entry_clone.text().to_string();
        let mut s = state_save.lock().unwrap();
        s.settings.ollama_endpoint = endpoint.clone();
        
        let final_url = normalize_url(&endpoint);
        if let Ok(url) = url::Url::parse(&final_url) {
            s.ollama = Ollama::from_url(url);
        }
        if let Err(e) = fs::write(&s.config_path, serde_json::to_string(&s.settings).unwrap()) {
            eprintln!("Failed to write settings.json: {}", e);
        }
    });
    general_box.append(&save_btn);
    settings_stack.add_titled(&general_box, Some("general"), "General");

    // --- Agents Settings ---
    let agents_box = Box::builder()
        .orientation(Orientation::Vertical)
        .margin_start(20)
        .margin_end(20)
        .margin_top(20)
        .spacing(10)
        .build();
    
    let agents_list = ListBox::builder().build();
    let scrolled_agents = ScrolledWindow::builder()
        .child(&agents_list)
        .vexpand(true)
        .build();
    agents_box.append(&scrolled_agents);

    let refresh_agents_list_func = {
        let state = state.clone();
        let agents_list = agents_list.clone();
        let agent_names_list = agent_names_list.clone();
        
        Rc::new(move || {
            while let Some(child) = agents_list.first_child() {
                agents_list.remove(&child);
            }
            refresh_agent_dropdown_func(state.clone(), agent_names_list.clone());
            let (agents, available_models) = {
                let s = state.lock().expect("Failed to lock state for agents list refresh");
                (s.settings.agents.clone(), s.available_models.clone())
            };
            for (idx, agent) in agents.into_iter().enumerate() {
                let row = Box::builder()
                    .orientation(Orientation::Vertical)
                    .spacing(5)
                    .margin_top(10)
                    .margin_bottom(10)
                    .build();

                row.append(&Label::builder().label("Agent Name").xalign(0.0).css_classes(["settings-label"]).build());
                let name_entry = Entry::builder().text(&agent.name).placeholder_text("Name").build();
                row.append(&name_entry);

                row.append(&Label::builder().label("Description").xalign(0.0).css_classes(["settings-label"]).build());
                let desc_entry = Entry::builder().text(&agent.description).placeholder_text("Description").build();
                row.append(&desc_entry);

                row.append(&Label::builder().label("Model").xalign(0.0).css_classes(["settings-label"]).build());
                
                let model_list = StringList::new(&[]);
                let model_refs: Vec<&str> = available_models.iter().map(|s| s.as_str()).collect();
                model_list.splice(0, 0, &model_refs);
                
                // If current model is not in list (or list empty), add it so user can see/save it
                let mut selected_idx = 0;
                let mut found = false;
                for (i, m) in available_models.iter().enumerate() {
                    if m == &agent.model {
                        selected_idx = i;
                        found = true;
                        break;
                    }
                }
                if !found && !agent.model.is_empty() {
                    model_list.append(&agent.model);
                    selected_idx = available_models.len();
                }

                let model_dropdown = DropDown::builder()
                    .model(&model_list)
                    .selected(selected_idx as u32)
                    .build();
                row.append(&model_dropdown);

                row.append(&Label::builder().label("System Prompt").xalign(0.0).css_classes(["settings-label"]).build());
                let prompt_entry = Entry::builder().text(&agent.system_prompt).placeholder_text("System Prompt").build();
                row.append(&prompt_entry);

                let actions_box = Box::builder().orientation(Orientation::Horizontal).spacing(10).margin_top(5).build();
                let save_btn = Button::with_label("Save");
                let delete_btn = Button::with_label("Delete");
                actions_box.append(&save_btn);
                actions_box.append(&delete_btn);
                row.append(&actions_box);
                row.append(&gtk::Separator::new(Orientation::Horizontal));

                let state_c = state.clone();
                let name_c = name_entry.clone();
                let desc_c = desc_entry.clone();
                let model_c = model_dropdown.clone();
                let prompt_c = prompt_entry.clone();
                let agent_names_list_c = agent_names_list.clone();
                save_btn.connect_clicked(move |_| {
                    let name = name_c.text().to_string();
                    let desc = desc_c.text().to_string();
                    let model = if let Some(item) = model_c.selected_item() {
                        item.downcast::<gtk::StringObject>().unwrap().string().to_string()
                    } else {
                        "".to_string()
                    };
                    let prompt = prompt_c.text().to_string();
                    
                    {
                        let mut s = state_c.lock().expect("Failed to lock state for saving agent");
                        if let Some(a) = s.settings.agents.get_mut(idx) {
                            a.name = name;
                            a.description = desc;
                            a.model = model;
                            a.system_prompt = prompt;
                            if let Err(e) = fs::write(&s.config_path, serde_json::to_string(&s.settings).expect("Failed to serialize settings")) {
                                eprintln!("Failed to write settings.json: {}", e);
                            }
                        }
                    }
                    refresh_agent_dropdown_func(state_c.clone(), agent_names_list_c.clone());
                });

                let state_d = state.clone();
                let agent_name_clone = agent.name.clone();
                let agents_list_clone = agents_list.clone();
                let row_clone = row.clone();
                let agent_names_list_d = agent_names_list.clone();
                delete_btn.connect_clicked(move |_| {
                    let mut s = state_d.lock().expect("Failed to lock state for deleting agent");
                    s.settings.agents.retain(|a| a.name != agent_name_clone);
                    if let Err(e) = fs::write(&s.config_path, serde_json::to_string(&s.settings).expect("Failed to serialize settings")) {
                        eprintln!("Failed to write settings.json: {}", e);
                    }
                    drop(s);
                    agents_list_clone.remove(&row_clone);
                    refresh_agent_dropdown_func(state_d.clone(), agent_names_list_d.clone());
                });
                agents_list.append(&row);
            }
        })
    };

    refresh_agents_list_func();

    let settings_stack_c = settings_stack.clone();
    let refresh_agents = refresh_agents_list_func.clone();
    settings_stack_c.connect_visible_child_name_notify(move |stack| {
        if stack.visible_child_name().as_deref() == Some("agents") {
            (refresh_agents)();
        }
    });

    let add_agent_btn = Button::with_label("Add Agent");
    let state_add = state.clone();
    let refresh_agents_add = refresh_agents_list_func.clone();
    add_agent_btn.connect_clicked(move |_| {
        let mut s = state_add.lock().expect("Failed to lock state for adding agent");
        s.settings.agents.push(Agent {
            name: "New Agent".to_string(),
            model: "llama3".to_string(),
            system_prompt: "You are a helpful assistant.".to_string(),
            description: "Personal Assistant".to_string(),
        });
        if let Err(e) = fs::write(&s.config_path, serde_json::to_string(&s.settings).expect("Failed to serialize settings")) {
            eprintln!("Failed to write settings.json: {}", e);
        }
        drop(s);
        refresh_agents_add();
    });

    let delete_chat_history_btn = Button::with_label("Delete Chat History");
    let state_delete_history = state.clone();
    delete_chat_history_btn.connect_clicked(move |_| {
        let mut s = state_delete_history.lock().unwrap();
        s.history.clear();
        if let Err(e) = fs::remove_file(&s.history_path) {
            eprintln!("Failed to remove history.json: {}", e);
        }
    });
    general_box.append(&delete_chat_history_btn);
    agents_box.append(&add_agent_btn);
    settings_stack.add_titled(&agents_box, Some("agents"), "Agents");

    // --- Models Settings ---
    let models_box = Box::builder()
        .orientation(Orientation::Vertical)
        .margin_start(20)
        .margin_end(20)
        .margin_top(20)
        .spacing(10)
        .build();

    models_box.append(&Label::builder().label("Pull Model").xalign(0.0).css_classes(["settings-title"]).build());
    
    let pull_box = Box::builder().orientation(Orientation::Horizontal).spacing(10).build();
    let pull_entry = Entry::builder().placeholder_text("Model name (e.g. llama3)").hexpand(true).build();
    let pull_btn = Button::with_label("Pull");
    pull_box.append(&pull_entry);
    pull_box.append(&pull_btn);
    models_box.append(&pull_box);

    let progress_label = Label::new(None);
    progress_label.set_visible(false);
    models_box.append(&progress_label);

    models_box.append(&gtk::Separator::new(Orientation::Horizontal));
    models_box.append(&Label::builder().label("Installed Models").xalign(0.0).css_classes(["settings-title"]).build());

    let models_list = ListBox::builder().build();
    let models_scrolled = ScrolledWindow::builder().child(&models_list).vexpand(true).build();
    models_box.append(&models_scrolled);

    let refresh_models_list = {
        let models_list = models_list.clone();
        let state = state.clone();
        Rc::new(move || {
            let models_list = models_list.clone();
            let state = state.clone();
            glib::MainContext::default().spawn_local(async move {
                let ollama = state.lock().unwrap().ollama.clone();
                if let Ok(models) = ollama.list_local_models().await {
                    {
                        let mut s = state.lock().unwrap();
                        s.available_models = models.iter().map(|m| m.name.clone()).collect();
                    }
                    while let Some(child) = models_list.first_child() {
                        models_list.remove(&child);
                    }
                    for model in models {
                        let row = Box::builder().orientation(Orientation::Horizontal).spacing(10).build();
                        let label = Label::builder().label(&model.name).xalign(0.0).hexpand(true).margin_start(10).margin_top(5).margin_bottom(5).build();
                        row.append(&label);
                        
                        let size_gb = model.size as f64 / 1024.0 / 1024.0 / 1024.0;
                        let size_label = Label::new(Some(&format!("{:.1} GB", size_gb)));
                        row.append(&size_label);
                        
                        models_list.append(&row);
                    }
                }
            });
        })
    };
    refresh_models_list();

    let state_pull = state.clone();
    let pull_entry_c = pull_entry.clone();
    let progress_label_c = progress_label.clone();
    let refresh_models_c = refresh_models_list.clone();
    pull_btn.connect_clicked(move |btn| {
        let model_name = pull_entry_c.text().to_string();
        if model_name.is_empty() { return; }
        
        btn.set_sensitive(false);
        progress_label_c.set_visible(true);
        progress_label_c.set_label(&format!("Pulling {}... this may take a while.", model_name));
        
        let state = state_pull.clone();
        let btn = btn.clone();
        let progress_label = progress_label_c.clone();
        let refresh = refresh_models_c.clone();
        
        glib::MainContext::default().spawn_local(async move {
            let ollama = state.lock().unwrap().ollama.clone();
            // Use simple pull for now
            let res = ollama.pull_model(model_name.clone(), false).await;
            
            btn.set_sensitive(true);
            match res {
                Ok(_) => {
                    progress_label.set_label(&format!("Successfully pulled {}", model_name));
                    refresh();
                }
                Err(e) => {
                    progress_label.set_label(&format!("Error: {}", e));
                }
            }
        });
    });

    settings_stack.add_titled(&models_box, Some("models"), "Models");

    // --- Personalization Settings ---
    let personalization_box = Box::builder()
        .orientation(Orientation::Vertical)
        .margin_start(20)
        .margin_end(20)
        .margin_top(20)
        .spacing(10)
        .build();
    
    personalization_box.append(&Label::builder()
        .label("Personalization")
        .xalign(0.0)
        .css_classes(["settings-title"])
        .build());

    personalization_box.append(&Label::builder()
        .label("Profiles")
        .xalign(0.0)
        .css_classes(["settings-title"])
        .build());

    let profiles_scrolled_content = Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(15)
        .build();
    let scrolled_profiles = ScrolledWindow::builder()
        .child(&profiles_scrolled_content)
        .vexpand(false)
        .min_content_height(120)
        .build();
    scrolled_profiles.add_css_class("profile-scrolled-window");
    personalization_box.append(&scrolled_profiles);

    personalization_box.append(&gtk::Separator::new(Orientation::Horizontal));

    // Editor Stack
    let editor_stack = Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .vexpand(true)
        .build();
    
    let empty_page = Box::builder()
        .orientation(Orientation::Vertical)
        .valign(gtk::Align::Center)
        .halign(gtk::Align::Center)
        .spacing(10)
        .build();
    empty_page.append(&Label::new(Some("Select a profile above to edit or activate.")));
    editor_stack.add_named(&empty_page, Some("empty"));

    let editor_page = Box::builder()
        .orientation(Orientation::Vertical)
        .spacing(10)
        .margin_top(10)
        .build();
    
    let edit_name = Entry::builder().placeholder_text("Profile Name").build();
    editor_page.append(&Label::builder().label("Profile Name").xalign(0.0).css_classes(["settings-label"]).build());
    editor_page.append(&edit_name);

    let edit_grid = gtk::Grid::builder().column_spacing(10).row_spacing(5).build();
    let edit_fname = Entry::builder().placeholder_text("First Name").hexpand(true).build();
    let edit_lname = Entry::builder().placeholder_text("Last Name").hexpand(true).build();
    let edit_email = Entry::builder().placeholder_text("Email").hexpand(true).build();
    let edit_phone = Entry::builder().placeholder_text("Phone").hexpand(true).build();
    
    edit_grid.attach(&edit_fname, 0, 0, 1, 1);
    edit_grid.attach(&edit_lname, 1, 0, 1, 1);
    edit_grid.attach(&edit_email, 0, 1, 1, 1);
    edit_grid.attach(&edit_phone, 1, 1, 1, 1);
    editor_page.append(&edit_grid);

    editor_page.append(&Label::builder().label("Location").xalign(0.0).css_classes(["settings-label"]).build());
    let edit_location = Entry::builder().placeholder_text("City, Country").build();
    editor_page.append(&edit_location);

    editor_page.append(&Label::builder().label("Bio / Context").xalign(0.0).css_classes(["settings-label"]).build());
    let edit_bio = Entry::builder().placeholder_text("Short bio").build();
    editor_page.append(&edit_bio);

    let actions_box = Box::builder().orientation(Orientation::Horizontal).spacing(10).margin_top(10).build();
    let activate_btn = Button::with_label("Use This Profile");
    let save_btn = Button::with_label("Save Changes");
    let delete_btn = Button::with_label("Delete Profile");
    let clear_mem_btn = Button::with_label("Clear Memory");
    
    delete_btn.add_css_class("destructive-action");
    clear_mem_btn.add_css_class("destructive-action");
    
    actions_box.append(&activate_btn);
    actions_box.append(&save_btn);
    actions_box.append(&delete_btn);
    actions_box.append(&clear_mem_btn);
    editor_page.append(&actions_box);

    editor_page.append(&Label::builder().label("Long-term Memory").xalign(0.0).css_classes(["settings-label"]).build());
    let memory_view = TextView::builder()
        .editable(false)
        .wrap_mode(gtk::WrapMode::WordChar)
        .height_request(150)
        .build();
    let memory_scroll = ScrolledWindow::builder().child(&memory_view).vexpand(true).build();
    editor_page.append(&memory_scroll);

    editor_stack.add_named(&editor_page, Some("editor"));
    personalization_box.append(&editor_stack);

    let selected_profile_idx: Rc<RefCell<Option<usize>>> = Rc::new(RefCell::new(None));

    let refresh_profiles_ui = {
        let state = state.clone();
        let profiles_list = profiles_scrolled_content.clone();
        let selected_idx = selected_profile_idx.clone();
        let editor_stack = editor_stack.clone();
        
        let edit_name = edit_name.clone();
        let edit_fname = edit_fname.clone();
        let edit_lname = edit_lname.clone();
        let edit_email = edit_email.clone();
        let edit_phone = edit_phone.clone();
        let edit_location = edit_location.clone();
        let edit_bio = edit_bio.clone();
        let activate_btn = activate_btn.clone();
        let memory_view = memory_view.clone();

        let refresh_ref: Rc<RefCell<Option<std::boxed::Box<dyn Fn()>>>> = Rc::new(RefCell::new(None));
        let refresh_ref_weak = refresh_ref.clone();

        let logic = move || {
            while let Some(child) = profiles_list.first_child() {
                profiles_list.remove(&child);
            }
            
            let (profiles, active_profile, memory_path) = {
                let s = state.lock().unwrap();
                (s.settings.profiles.clone(), s.settings.active_profile.clone(), s.memory_path.clone())
            };

            let current_sel = *selected_idx.borrow();

            for (idx, profile) in profiles.iter().enumerate() {
                let circle_btn = Button::builder()
                    .css_classes(["profile-circle"])
                    .width_request(80)
                    .height_request(80)
                    .build();
                
                if let Some(active) = &active_profile {
                    if active == &profile.name {
                        circle_btn.add_css_class("active-profile");
                    }
                }
                
                if Some(idx) == current_sel {
                     circle_btn.add_css_class("selected-editing");
                }

                let icon_label = Label::new(Some(&profile.name.chars().next().unwrap_or('?').to_string().to_uppercase()));
                circle_btn.set_child(Some(&icon_label));
                
                let container = Box::builder().orientation(Orientation::Vertical).spacing(5).build();
                container.append(&circle_btn);
                container.append(&Label::builder().label(&profile.name).css_classes(["profile-mini-name"]).build());
                profiles_list.append(&container);

                let sel_idx = selected_idx.clone();
                let refresh = refresh_ref_weak.clone();
                circle_btn.connect_clicked(move |_| {
                    *sel_idx.borrow_mut() = Some(idx);
                    if let Some(f) = &*refresh.borrow() { f(); }
                });
            }

            // Add Profile Button
            let add_btn = Button::builder().label("+").css_classes(["profile-circle"]).width_request(80).height_request(80).build();
            let state_add = state.clone();
            let refresh_add = refresh_ref_weak.clone();
            let sel_add = selected_idx.clone();
            add_btn.connect_clicked(move |_| {
                {
                    let mut s = state_add.lock().unwrap();
                    s.settings.profiles.push(Profile {
                        id: glib::uuid_string_random().to_string(),
                        name: "New Profile".to_string(),
                        first_name: "".to_string(),
                        last_name: "".to_string(),
                        email: "".to_string(),
                        phone: "".to_string(),
                        location: "".to_string(),
                        bio: "".to_string(),
                        image_path: None,
                    });
                    let _ = fs::write(&s.config_path, serde_json::to_string(&s.settings).unwrap());
                    *sel_add.borrow_mut() = Some(s.settings.profiles.len() - 1);
                }
                if let Some(f) = &*refresh_add.borrow() { f(); }
            });
            let container = Box::builder().orientation(Orientation::Vertical).spacing(5).build();
            container.append(&add_btn);
            container.append(&Label::new(Some("Add")));
            profiles_list.append(&container);

            if let Some(idx) = current_sel {
                if let Some(profile) = profiles.get(idx) {
                    editor_stack.set_visible_child_name("editor");
                    edit_name.set_text(&profile.name);
                    edit_fname.set_text(&profile.first_name);
                    edit_lname.set_text(&profile.last_name);
                    edit_email.set_text(&profile.email);
                    edit_phone.set_text(&profile.phone);
                    edit_location.set_text(&profile.location);
                    edit_bio.set_text(&profile.bio);

                    // Load Memory
                    let mem_file = memory_path.join(format!("{}.txt", profile.id));
                    let memory = fs::read_to_string(mem_file).unwrap_or_default();
                    memory_view.buffer().set_text(&memory);
                    
                    if let Some(active) = &active_profile {
                        if active == &profile.name {
                            activate_btn.set_label("Current Profile");
                            activate_btn.set_sensitive(false);
                        } else {
                            activate_btn.set_label("Use This Profile");
                            activate_btn.set_sensitive(true);
                        }
                    } else {
                        activate_btn.set_label("Use This Profile");
                        activate_btn.set_sensitive(true);
                    }
                } else {
                    *selected_idx.borrow_mut() = None;
                    editor_stack.set_visible_child_name("empty");
                }
            } else {
                editor_stack.set_visible_child_name("empty");
            }
        };
        
        *refresh_ref.borrow_mut() = Some(std::boxed::Box::new(logic));
        refresh_ref
    };

    let call_refresh = {
        let refresh = refresh_profiles_ui.clone();
        move || { if let Some(f) = &*refresh.borrow() { f(); } }
    };
    call_refresh();

    let state_save = state.clone();
    let sel_save = selected_profile_idx.clone();
    let refresh_save = call_refresh.clone();
    let name_s = edit_name.clone();
    let fname_s = edit_fname.clone();
    let lname_s = edit_lname.clone();
    let email_s = edit_email.clone();
    let phone_s = edit_phone.clone();
    let loc_s = edit_location.clone();
    let bio_s = edit_bio.clone();

    save_btn.connect_clicked(move |_| {
        if let Some(idx) = *sel_save.borrow() {
            let mut s = state_save.lock().unwrap();
            if let Some(p) = s.settings.profiles.get_mut(idx) {
                p.name = name_s.text().to_string();
                p.first_name = fname_s.text().to_string();
                p.last_name = lname_s.text().to_string();
                p.email = email_s.text().to_string();
                p.phone = phone_s.text().to_string();
                p.location = loc_s.text().to_string();
                p.bio = bio_s.text().to_string();
                let _ = fs::write(&s.config_path, serde_json::to_string(&s.settings).unwrap());
            }
        }
        refresh_save();
    });

    let state_act = state.clone();
    let sel_act = selected_profile_idx.clone();
    let refresh_act = call_refresh.clone();
    activate_btn.connect_clicked(move |_| {
        if let Some(idx) = *sel_act.borrow() {
            let mut s = state_act.lock().unwrap();
            if let Some(p) = s.settings.profiles.get(idx) {
                s.settings.active_profile = Some(p.name.clone());
                let _ = fs::write(&s.config_path, serde_json::to_string(&s.settings).unwrap());
            }
        }
        refresh_act();
    });

    let state_del = state.clone();
    let sel_del = selected_profile_idx.clone();
    let refresh_del = call_refresh.clone();
    delete_btn.connect_clicked(move |_| {
        if let Some(idx) = *sel_del.borrow() {
            let mut s = state_del.lock().unwrap();
            if idx < s.settings.profiles.len() {
                let name = s.settings.profiles[idx].name.clone();
                s.settings.profiles.remove(idx);
                if s.settings.active_profile.as_ref() == Some(&name) {
                    s.settings.active_profile = None;
                }
                let _ = fs::write(&s.config_path, serde_json::to_string(&s.settings).unwrap());
            }
        }
        *sel_del.borrow_mut() = None;
        refresh_del();
    });

    let state_clr = state.clone();
    let sel_clr = selected_profile_idx.clone();
    let refresh_clr = call_refresh.clone();
    clear_mem_btn.connect_clicked(move |_| {
        if let Some(idx) = *sel_clr.borrow() {
            let s = state_clr.lock().unwrap();
            if let Some(p) = s.settings.profiles.get(idx) {
                let mem_file = s.memory_path.join(format!("{}.txt", p.id));
                let _ = fs::remove_file(mem_file);
            }
        }
        refresh_clr();
    });

    let personalization_scrolled = ScrolledWindow::builder()
        .child(&personalization_box)
        .vexpand(true)
        .build();

    settings_stack.add_titled(&personalization_scrolled, Some("personalization"), "Personalization");

    scrolled_profiles.add_css_class("profile-scrolled-window");

    main_stack.add_titled(&chat_box_container, Some("chat"), "Chat");
    main_stack.add_titled(&settings_view, Some("settings"), "Settings");

    let window = ApplicationWindow::builder()
        .application(app)
        .title("Arch LLM")
        .default_width(1200)
        .default_height(800)
        .child(&root_stack)
        .build();

    let main_stack_clone = main_stack.clone();
    settings_btn.connect_clicked(move |_| {
        main_stack_clone.set_visible_child_name("settings");
    });

    let main_stack_clone = main_stack.clone();
    back_btn.connect_clicked(move |_| {
        main_stack_clone.set_visible_child_name("chat");
    });

    // --- History Helper ---
    let refresh_history: Rc<RefCell<Option<std::boxed::Box<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    
    let refresh_history_impl = {
        let state = state.clone();
        let history_list = history_list.clone();
        let render_chat = render_chat.clone();
        let refresh_history_ref = refresh_history.clone();
        move || {
            while let Some(child) = history_list.first_child() {
                history_list.remove(&child);
            }
            let history = {
                let s = state.lock().unwrap();
                s.history.clone()
            };
            for item in history.into_iter().rev() {
                let row_btn = Button::builder()
                    .label(&item.title)
                    .css_classes(["history-item"])
                    .build();
                
                let state_h = state.clone();
                let render_chat = render_chat.clone();
                let item_messages = item.messages.clone();
                row_btn.connect_clicked(move |_| {
                    let mut s = state_h.lock().unwrap();
                    s.messages = item_messages.clone();
                    render_chat(&s.messages);
                });

                // Context Menu
                let popover = Popover::new();
                let menu_box = Box::builder().orientation(Orientation::Vertical).spacing(5).margin_top(10).margin_bottom(10).margin_start(10).margin_end(10).build();
                
                let rename_box = Box::builder().orientation(Orientation::Horizontal).spacing(5).build();
                let rename_entry = Entry::builder().text(&item.title).hexpand(true).build();
                let rename_confirm_btn = Button::with_label("Save");
                rename_box.append(&rename_entry);
                rename_box.append(&rename_confirm_btn);
                menu_box.append(&rename_box);

                let delete_btn = Button::with_label("Delete Chat");
                delete_btn.add_css_class("destructive-action"); // Will add CSS later
                menu_box.append(&delete_btn);
                
                popover.set_child(Some(&menu_box));
                popover.set_parent(&row_btn);
                popover.set_has_arrow(false);

                let gesture = GestureClick::new();
                gesture.set_button(3); // Right click
                gesture.connect_pressed(glib::clone!(#[weak] popover, #[weak] row_btn, move |_, _, _, _| {
                     let allocation = row_btn.allocation();
                     popover.set_pointing_to(Some(&allocation));
                     popover.popup();
                }));
                row_btn.add_controller(gesture);

                // Handlers
                let state_r = state.clone();
                let item_id = item.id.clone();
                let refresh_r = refresh_history_ref.clone();
                let rename_entry_c = rename_entry.clone();
                let popover_r = popover.clone();
                
                rename_confirm_btn.connect_clicked(move |_| {
                    let new_title = rename_entry_c.text().to_string();
                    if new_title.is_empty() { return; }
                    {
                        let mut s = state_r.lock().unwrap();
                        if let Some(h) = s.history.iter_mut().find(|x| x.id == item_id) {
                            h.title = new_title;
                            if let Err(e) = fs::write(&s.history_path, serde_json::to_string(&s.history).unwrap()) {
                                eprintln!("Failed to save history: {}", e);
                            }
                        }
                    }
                    popover_r.popdown();
                    if let Some(f) = &*refresh_r.borrow() { f(); }
                });

                let state_d = state.clone();
                let item_id_d = item.id.clone();
                let refresh_d = refresh_history_ref.clone();
                let popover_d = popover.clone();
                
                delete_btn.connect_clicked(move |_| {
                    {
                        let mut s = state_d.lock().unwrap();
                        s.history.retain(|x| x.id != item_id_d);
                        if let Err(e) = fs::write(&s.history_path, serde_json::to_string(&s.history).unwrap()) {
                            eprintln!("Failed to save history: {}", e);
                        }
                        // If deleted chat was active, clear it? Maybe not necessary for UX flow
                    }
                    popover_d.popdown();
                    if let Some(f) = &*refresh_d.borrow() { f(); }
                });

                history_list.append(&row_btn);
            }
        }
    };
    *refresh_history.borrow_mut() = Some(std::boxed::Box::new(refresh_history_impl));
    if let Some(f) = &*refresh_history.borrow() { f(); }

    new_chat_btn.connect_clicked({
        let state = state.clone();
        let render_chat = render_chat.clone();
        move |_| {
            let mut s = state.lock().unwrap();
            s.messages.clear();
            render_chat(&s.messages);
        }
    });

    // --- Event Handlers ---
    let state_clone = state.clone();
    let render_chat_clone = render_chat.clone();
    agent_dropdown.connect_selected_notify(move |dd| {
        let mut s = state_clone.lock().unwrap();
        s.current_agent_idx = dd.selected() as usize;
        s.messages.clear();
        render_chat_clone(&s.messages);
    });

    let state_clone = state.clone();
    let chat_box_clone = chat_box.clone();
    let refresh_history_clone = refresh_history.clone();
    let send_btn_clone = send_btn.clone();
    let text_view_clone = text_view.clone();
    let scroll_to_bottom_clone = scroll_to_bottom.clone();

    // Logic to handle Send / Stop
    let handle_send_or_stop = move || {
        let is_sending = send_btn_clone.label().map(|l| l.as_str() == "Stop").unwrap_or(false);

        if is_sending {
            // STOP Logic
            let mut s = state_clone.lock().unwrap();
            if let Some(handle) = s.current_task.take() {
                handle.abort();
            }
            send_btn_clone.set_label("Send");
            send_btn_clone.remove_css_class("stop-btn");
            send_btn_clone.add_css_class("send-btn");
            return;
        }

        // SEND Logic
        let buffer = text_view_clone.buffer();
        let (start, end) = buffer.bounds();
        let text = buffer.text(&start, &end, false).to_string();
        
        if text.trim().is_empty() { return; }
        buffer.set_text("");

        send_btn_clone.set_label("Stop");
        send_btn_clone.remove_css_class("send-btn");
        send_btn_clone.add_css_class("stop-btn");

        // Add user message to UI
        let user_label = Label::builder()
            .xalign(0.0)
            .wrap(true)
            .css_classes(["user-message"])
            .halign(gtk::Align::End)
            .build();
        user_label.set_markup(&glib::markup_escape_text(&text));
        chat_box_clone.append(&user_label);
        scroll_to_bottom_clone();

        // Response container
        let bot_msg_box = Box::builder().orientation(Orientation::Horizontal).spacing(10).build();
        let bot_spinner = Spinner::builder().spinning(true).build();
        let bot_label = Label::builder()
            .label("Thinking...")
            .xalign(0.0)
            .wrap(true)
            .css_classes(["bot-message"])
            .hexpand(true)
            .build();
        bot_msg_box.append(&bot_spinner);
        bot_msg_box.append(&bot_label);
        chat_box_clone.append(&bot_msg_box);
        scroll_to_bottom_clone();

        let (sender, receiver) = async_channel::unbounded();
        
        // Receiver (Main Thread)
        let mut full_response_acc = String::new();
        let bot_label_c = bot_label.clone();
        let bot_spinner_c = bot_spinner.clone();
        let scroll_to_bottom_c = scroll_to_bottom_clone.clone();
        let send_btn_c = send_btn_clone.clone();
        let state_c = state_clone.clone();
        let text_c = text.clone();
        let refresh_history_c = refresh_history_clone.clone();
        let sender_for_title = sender.clone();

        glib::MainContext::default().spawn_local(async move {
            while let Ok(event) = receiver.recv().await {
                match event {
                    ChatEvent::Chunk(chunk) => {
                        bot_spinner_c.set_spinning(false);
                        bot_spinner_c.set_visible(false);
                        full_response_acc.push_str(&chunk);
                        bot_label_c.set_markup(&markdown_to_pango(&full_response_acc));
                        scroll_to_bottom_c();
                    }
                    ChatEvent::Error(err) => {
                        bot_label_c.set_label(&format!("Error: {}", err));
                        send_btn_c.set_label("Send");
                        send_btn_c.remove_css_class("stop-btn");
                        send_btn_c.add_css_class("send-btn");
                        
                        let mut s = state_c.lock().unwrap();
                        s.current_task = None;
                        break;
                    }
                    ChatEvent::RefreshHistory => {
                        if let Some(f) = &*refresh_history_c.borrow() { f(); }
                    }
                    ChatEvent::Done(full_text) => {
                        // Save history
                        let is_first_message;
                        let history_id = glib::uuid_string_random().to_string();
                        let (history_path, ollama_clone, model_clone) = {
                            let mut s = state_c.lock().unwrap();
                            s.messages.push(ChatMessage::assistant(full_text));
                            is_first_message = s.messages.len() <= 3;
                            s.current_task = None;
                            
                            let history_item = ChatHistory {
                                id: history_id.clone(),
                                title: text_c.chars().take(20).collect(),
                                messages: s.messages.clone(),
                            };
                            s.history.push(history_item);
                            if let Err(e) = fs::write(&s.history_path, serde_json::to_string(&s.history).unwrap()) {
                                eprintln!("Failed to write history.json: {}", e);
                            }
                            
                            // Need copies for async title gen
                            let agent = s.settings.agents.get(s.current_agent_idx).cloned().unwrap_or_else(|| s.settings.agents[0].clone());
                            (s.history_path.clone(), s.ollama.clone(), agent.model.clone())
                        };

                        // Reset UI
                        send_btn_c.set_label("Send");
                        send_btn_c.remove_css_class("stop-btn");
                        send_btn_c.add_css_class("send-btn");
                        if let Some(f) = &*refresh_history_c.borrow() { f(); }

                        // Generate Title Async
                        if is_first_message {
                            let state_title = state_c.clone();
                            let user_text_title = text_c.clone();
                            let sender_title = sender_for_title.clone();
                            
                            tokio::spawn(async move {
                                let title_prompt = format!(
                                    "Generate a very short, creative 2-4 word title for a chat that starts with: \"{}\". Output ONLY the title, no quotes or punctuation.",
                                    user_text_title
                                );
                                let req = ChatMessageRequest::new(
                                    model_clone,
                                    vec![ChatMessage::user(title_prompt)]
                                );
                                
                                if let Ok(res) = ollama_clone.send_chat_messages(req).await {
                                    let new_title = res.message.content.trim().trim_matches('"').trim_matches('.').to_string();
                                    if !new_title.is_empty() {
                                        let mut s = state_title.lock().unwrap();
                                        if let Some(hist) = s.history.iter_mut().find(|h| h.id == history_id) {
                                            hist.title = new_title;
                                            if let Err(e) = fs::write(&history_path, serde_json::to_string(&s.history).unwrap()) {
                                                eprintln!("Failed to write history.json: {}", e);
                                            }
                                        }
                                    }
                                    let _ = sender_title.send(ChatEvent::RefreshHistory).await;
                                }
                            });
                        }
                        // Do NOT break here, as we might receive RefreshHistory later
                    }
                }
            }
        });

        // Task (Tokio Thread)
        let state = state_clone.clone();
        let text_task = text.clone();
        
        let task = tokio::spawn(async move {
            let (ollama, model, messages, profile_id, memory_path) = {
                let mut s = state.lock().unwrap();
                let agent = s.settings.agents.get(s.current_agent_idx).cloned().unwrap_or_else(|| s.settings.agents[0].clone());
                
                let mut profile_info = None;
                if let Some(active_name) = &s.settings.active_profile {
                    if let Some(profile) = s.settings.profiles.iter().find(|p| &p.name == active_name) {
                        profile_info = Some((profile.id.clone(), profile.first_name.clone(), profile.last_name.clone(), profile.location.clone(), profile.bio.clone()));
                    }
                }

                if s.messages.is_empty() {
                    let mut system_prompt = agent.system_prompt.clone();
                    
                    if let Some((id, fname, lname, loc, bio)) = &profile_info {
                        system_prompt.push_str("\n\n---\nUser Profile:\n");
                        if !fname.is_empty() || !lname.is_empty() {
                            system_prompt.push_str(&format!("Name: {} {}\n", fname, lname));
                        }
                        if !loc.is_empty() {
                            system_prompt.push_str(&format!("Location: {}\n", loc));
                        }
                        if !bio.is_empty() {
                            system_prompt.push_str(&format!("Bio: {}\n", bio));
                        }

                        // Load Long-term Memory
                        let mem_file = s.memory_path.join(format!("{}.txt", id));
                        if let Ok(memory) = fs::read_to_string(&mem_file) {
                            if !memory.trim().is_empty() {
                                system_prompt.push_str("\nLong-term Memory of User:\n");
                                system_prompt.push_str(&memory);
                            }
                        }
                    }
                    s.messages.push(ChatMessage::system(system_prompt));
                }
                
                s.messages.push(ChatMessage::user(text_task.clone()));
                (s.ollama.clone(), agent.model.clone(), s.messages.clone(), profile_info.map(|p| p.0), s.memory_path.clone())
            };

            match ollama.send_chat_messages_stream(
                ChatMessageRequest::new(model.clone(), messages.clone())
            ).await {
                Ok(mut stream) => {
                    let mut full_response = String::new();
                    while let Some(res) = stream.next().await {
                        if let Ok(res) = res {
                            let msg = res.message;
                            full_response.push_str(&msg.content);
                            if sender.send(ChatEvent::Chunk(msg.content)).await.is_err() { break; }
                        }
                    }
                    
                    // Update Memory if profile is active
                    if let Some(id) = profile_id {
                        let ollama_mem = ollama.clone();
                        let model_mem = model.clone();
                        let mut messages_mem = messages.clone();
                        messages_mem.push(ChatMessage::assistant(full_response.clone()));
                        let memory_path_mem = memory_path.clone();

                        tokio::spawn(async move {
                            let mem_file = memory_path_mem.join(format!("{}.txt", id));
                            let existing_memory = fs::read_to_string(&mem_file).unwrap_or_default();
                            
                            let memory_prompt = format!(
                                "You are a memory module. Based on the recent conversation above and the existing knowledge about the user, update the Long-term Memory. \
                                Existing Knowledge:\n{}\n\n\
                                Requirements:\n\
                                1. Output a concise, bulleted list of facts, preferences, and important context about the user.\n\
                                2. Include new info from this chat.\n\
                                3. Keep it brief and relevant for future assistance.\n\
                                4. Output ONLY the list, no headers or conversational text.",
                                existing_memory
                            );
                            
                            messages_mem.push(ChatMessage::user(memory_prompt));
                            if let Ok(res) = ollama_mem.send_chat_messages(ChatMessageRequest::new(model_mem, messages_mem)).await {
                                let new_memory = res.message.content.trim().to_string();
                                if !new_memory.is_empty() {
                                    let _ = fs::write(mem_file, new_memory);
                                }
                            }
                        });
                    }

                    let _ = sender.send(ChatEvent::Done(full_response)).await;
                }
                Err(e) => {
                    let _ = sender.send(ChatEvent::Error(format!("{:?}", e))).await;
                }
            }
        });
        
        let mut s = state_clone.lock().unwrap();
        s.current_task = Some(task.abort_handle());
    };

    let handle_send_clone = handle_send_or_stop.clone();
    send_btn.connect_clicked(move |_| {
        handle_send_clone();
    });

    // Key controller for Shift+Enter vs Enter
    let controller = gtk::EventControllerKey::new();
    controller.connect_key_pressed(move |_, key, _, modifiers| {
        if key == gtk::gdk::Key::Return && !modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK) {
            handle_send_or_stop();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    text_view.add_controller(controller);

    // Global Shortcuts
    let controller = EventControllerKey::new();
    let new_chat_btn_c = new_chat_btn.clone();
    let settings_btn_c = settings_btn.clone();
    let app_c = app.clone();
    
    controller.connect_key_pressed(move |_, key, _, modifiers| {
        if modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK) {
            match key {
                gtk::gdk::Key::n => {
                    new_chat_btn_c.emit_clicked();
                    return glib::Propagation::Stop;
                }
                gtk::gdk::Key::comma => {
                    settings_btn_c.emit_clicked();
                    return glib::Propagation::Stop;
                }
                gtk::gdk::Key::q => {
                    app_c.quit();
                    return glib::Propagation::Stop;
                }
                _ => {}
            }
        }
        glib::Propagation::Proceed
    });
    window.add_controller(controller);

    // Load CSS
    let provider = gtk::CssProvider::new();
    provider.load_from_data(r#"
        .msg-header {
            font-weight: bold;
            font-size: 12px;
            color: #aaa;
            margin-bottom: 2px;
        }
        .code-frame {
            background-color: #1e1f20;
            border-radius: 8px;
            border: 1px solid #333;
        }
        .code-view {
            font-family: monospace;
            padding: 10px;
        }
        .destructive-action {
            color: #ff5555;
        }
        .destructive-action:hover {
            background-color: rgba(255, 85, 85, 0.1);
        }

        window { background-color: #131314; color: #e3e3e3; font-family: sans-serif; }
        .sidebar { background-color: #1e1f20; }
        .sidebar button {
            background: none;
            border: none;
            color: #e3e3e3;
            padding: 10px 15px;
            border-radius: 20px;
        }
        .sidebar button:hover { background-color: #333537; }

        .history-list { background: none; }
        .history-item {
            margin: 2px 10px;
            padding: 8px 15px;
            border-radius: 10px;
            font-size: 14px;
        }
        
        textview.chat-input {
            background-color: #1e1f20;
            border-radius: 15px;
            color: white;
            padding: 10px;
            font-size: 16px;
        }
        
        entry {
            background-color: #1e1f20;
            border-radius: 28px;
            padding: 12px 20px;
            color: white;
            border: 1px solid #444;
            font-size: 16px;
        }
        
        dropdown {
            background: none;
            border: none;
            color: #e3e3e3;
            font-weight: bold;
        }

        .user-message {
            font-weight: 500;
            margin-top: 10px;
            margin-bottom: 10px;
            font-size: 16px;
            color: #fff;
            background-color: #0b93f6;
            padding: 10px 15px;
            border-radius: 18px;
        }
        .bot-message {
            line-height: 1.6;
            font-size: 16px;
            color: #e3e3e3;
            margin-bottom: 20px;
        }
        .settings-title {
            font-size: 20px;
            font-weight: bold;
            margin-bottom: 10px;
        }
        .settings-label {
            font-weight: bold;
            margin-top: 10px;
            color: #aaa;
            font-size: 12px;
            text-transform: uppercase;
        }
        .profile-circle {
            border-radius: 50%;
            background-color: #333537;
            border: 2px solid #444;
            padding: 0;
            min-width: 80px;
            min-height: 80px;
        }
        .profile-circle:hover {
            background-color: #444;
            border-color: #0b93f6;
        }
        .active-profile {
            border-color: #0b93f6;
            border-width: 3px;
        }
        .selected-editing {
            background-color: #0b93f6;
            color: white;
        }
        .profile-circle-label {
            font-size: 24px;
            font-weight: bold;
            color: #fff;
        }
        .profile-mini-name {
            font-size: 12px;
            color: #aaa;
        }
        .profile-scrolled-window {
            min-height: 150px;
        }
        
        .send-btn {
            background-color: #0b93f6;
            color: white;
            border-radius: 50%;
            min-width: 40px;
            min-height: 40px;
            font-weight: bold;
            padding: 0;
        }
        .stop-btn {
            background-color: #e53935;
            color: white;
            border-radius: 50%;
            min-width: 40px;
            min-height: 40px;
            font-weight: bold;
            padding: 0;
        }
        tt {
            font-family: monospace;
            background-color: #2b2d30;
            padding: 2px 5px;
            border-radius: 4px;
        }
        
        .welcome-icon {
            font-size: 64px;
            margin-bottom: 10px;
        }
        .welcome-text {
            font-size: 18px;
            color: #888;
            font-weight: bold;
        }
    "#);
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().expect("Could not connect to a display."),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    // Connection Check
    let root_stack_c = root_stack.clone();
    let state_conn = state.clone();
    
    // Set initial state
    root_stack_c.set_visible_child_name("loading");
    
    // Retry / Setup handler
    let endpoint_entry_setup_c = endpoint_entry_setup.clone();
    let endpoint_entry_general_c = endpoint_entry.clone();
    retry_btn.connect_clicked(glib::clone!(#[weak] root_stack_c, #[weak] state_conn, move |_| {
        let new_endpoint = endpoint_entry_setup_c.text().to_string();
        
        {
            let mut s = state_conn.lock().unwrap();
            s.settings.ollama_endpoint = new_endpoint.clone();
            let final_url = normalize_url(&new_endpoint);
            if let Ok(url) = url::Url::parse(&final_url) {
                s.ollama = Ollama::from_url(url);
            }
            // Update general settings entry too
            endpoint_entry_general_c.set_text(&new_endpoint);
            
            // Save settings
            if let Err(e) = fs::write(&s.config_path, serde_json::to_string(&s.settings).unwrap()) {
                eprintln!("Failed to write settings.json: {}", e);
            }
        }

        root_stack_c.set_visible_child_name("loading");
        let root_stack_c = root_stack_c.clone();
        let state = state_conn.clone();
        glib::MainContext::default().spawn_local(async move {
            let ollama = state.lock().unwrap().ollama.clone();
            match ollama.list_local_models().await {
                Ok(models) => {
                    {
                        let mut s = state.lock().unwrap();
                        s.available_models = models.into_iter().map(|m| m.name).collect();
                    }
                    root_stack_c.set_visible_child_name("main");
                }
                Err(_) => {
                    root_stack_c.set_visible_child_name("error");
                }
            }
        });
    }));

    // Trigger check
    glib::MainContext::default().spawn_local(async move {
        let ollama = state_conn.lock().unwrap().ollama.clone();
        match ollama.list_local_models().await {
            Ok(models) => {
                {
                    let mut s = state_conn.lock().unwrap();
                    s.available_models = models.into_iter().map(|m| m.name).collect();
                }
                root_stack_c.set_visible_child_name("main");
            }
            Err(_) => {
                root_stack_c.set_visible_child_name("error");
            }
        }
    });

    window.present();
}
