use gtk4 as gtk;
use gtk::glib;
use gtk::prelude::*;
use std::rc::Rc;
use std::cell::RefCell;
use gtk::{
    Application, ApplicationWindow, Box, Orientation, Label, Entry, Button,
    ScrolledWindow, ListBox, DropDown, StringList, Stack, StackSidebar
};
use std::sync::{Arc, Mutex};
use serde::{Serialize, Deserialize};
use std::fs;
use ollama_rs::generation::chat::request::ChatMessageRequest;
use ollama_rs::generation::chat::ChatMessage;
use ollama_rs::Ollama;
use futures_util::StreamExt;
use pulldown_cmark::{Parser, Options, Tag, TagEnd, Event};

#[derive(Serialize, Deserialize, Clone)]
struct Agent {
    name: String,
    model: String,
    system_prompt: String,
    description: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct Profile {
    name: String,
    first_name: String,
    last_name: String,
    email: String,
    phone: String,
    location: String,
    bio: String,
    image_path: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
struct Settings {
    ollama_endpoint: String,
    agents: Vec<Agent>,
    #[serde(default)]
    profiles: Vec<Profile>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            ollama_endpoint: "http://localhost:11434".to_string(),
            agents: vec![
                Agent {
                    name: "Default Assistant".to_string(),
                    model: "llama3".to_string(),
                    system_prompt: "You are a helpful assistant.".to_string(),
                    description: "Standard personal assistant".to_string(),
                }
            ],
            profiles: Vec::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
struct ChatHistory {
    id: String,
    title: String,
    messages: Vec<ChatMessage>,
}

struct AppState {
    ollama: Ollama,
    current_agent_idx: usize,
    messages: Vec<ChatMessage>,
    history: Vec<ChatHistory>,
    settings: Settings,
}

fn markdown_to_pango(markdown: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    let parser = Parser::new_ext(markdown, options);
    let mut pango_markup = String::new();

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Strong => pango_markup.push_str("<b>"),
                Tag::Emphasis => pango_markup.push_str("<i>"),
                Tag::Strikethrough => pango_markup.push_str("<s>"),
                Tag::CodeBlock(_) => pango_markup.push_str("\n<tt>"),
                Tag::Heading { level, .. } => {
                    let size = match level {
                        pulldown_cmark::HeadingLevel::H1 => "xx-large",
                        pulldown_cmark::HeadingLevel::H2 => "x-large",
                        _ => "large",
                    };
                    pango_markup.push_str(&format!("\n<span font_size='{}' weight='bold'>", size));
                }
                Tag::Link { .. } => pango_markup.push_str("<u>"),
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Strong => pango_markup.push_str("</b>"),
                TagEnd::Emphasis => pango_markup.push_str("</i>"),
                TagEnd::Strikethrough => pango_markup.push_str("</s>"),
                TagEnd::CodeBlock => pango_markup.push_str("</tt>\n"),
                TagEnd::Heading(_) => pango_markup.push_str("</span>\n"),
                TagEnd::Link => pango_markup.push_str("</u>"),
                _ => {}
            },
            Event::Text(text) => pango_markup.push_str(&glib::markup_escape_text(&text)),
            Event::Code(code) => pango_markup.push_str(&format!("<tt>{}</tt>", glib::markup_escape_text(&code))),
            Event::SoftBreak | Event::HardBreak => pango_markup.push('\n'),
            Event::Rule => pango_markup.push_str("\n───────────────────\n"),
            _ => {}
        }
    }
    pango_markup
}

#[tokio::main]
async fn main() -> glib::ExitCode {
    let app = Application::builder()
        .application_id("org.archllm.ollama_chat")
        .build();

    app.connect_activate(build_ui);
    app.run()
}

fn build_ui(app: &Application) {
    let history_data = fs::read_to_string("history.json")
        .ok()
        .and_then(|s| serde_json::from_str::<Vec<ChatHistory>>(&s).ok())
        .unwrap_or_default();

    let settings_data = fs::read_to_string("settings.json")
        .ok()
        .and_then(|s| serde_json::from_str::<Settings>(&s).ok())
        .unwrap_or_else(|| Settings::default());

    let parse_url = |s: &str| {
        let mut s = s.trim().to_string();
        if !s.starts_with("http://") && !s.starts_with("https://") {
            s = format!("http://{}", s);
        }
        url::Url::parse(&s).unwrap_or_else(|_| url::Url::parse("http://localhost:11434").unwrap())
    };

    let ollama = Ollama::from_url(parse_url(&settings_data.ollama_endpoint));

    let state = Arc::new(Mutex::new(AppState {
        ollama,
        current_agent_idx: 0,
        messages: Vec::new(),
        history: history_data,
        settings: settings_data,
    }));

    let main_stack = Stack::builder()
        .transition_type(gtk::StackTransitionType::SlideLeftRight)
        .build();

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

    let refresh_agent_dropdown = {
        let state = state.clone();
        let agent_names_list = agent_names_list.clone();
        move || {
            let names: Vec<String> = {
                let s = match state.try_lock() {
                    Ok(s) => s,
                    Err(_) => {
                        eprintln!("Deadlock prevented in refresh_agent_dropdown");
                        return;
                    }
                };
                s.settings.agents.iter().map(|a| a.name.clone()).collect()
            };
            let name_refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
            agent_names_list.splice(0, agent_names_list.n_items(), &name_refs);
        }
    };

    refresh_agent_dropdown();
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

    // Input area
    let input_container = Box::builder()
        .orientation(Orientation::Vertical)
        .margin_bottom(30)
        .margin_start(100)
        .margin_end(100)
        .build();

    let entry = Entry::builder()
        .placeholder_text("Ask Ollama...")
        .height_request(50)
        .build();
    
    input_container.append(&entry);
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
        
        let mut final_url = endpoint.trim().to_string();
        if !final_url.starts_with("http://") && !final_url.starts_with("https://") {
            final_url = format!("http://{}", final_url);
        }
        if let Ok(url) = url::Url::parse(&final_url) {
            s.ollama = Ollama::from_url(url);
        }
        let _ = fs::write("settings.json", serde_json::to_string(&s.settings).unwrap());
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

    let refresh_agents_list = {
        let state = state.clone();
        let agents_list = agents_list.clone();
        let refresh_dropdown = refresh_agent_dropdown.clone();
        move || {
            while let Some(child) = agents_list.first_child() {
                agents_list.remove(&child);
            }
            refresh_dropdown();
            let agents = {
                let s = match state.try_lock() {
                    Ok(s) => s,
                    Err(_) => {
                        eprintln!("Deadlock prevented in refresh_agents_list");
                        return;
                    }
                };
                s.settings.agents.clone()
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
                let model_entry = Entry::builder().text(&agent.model).placeholder_text("Model").build();
                row.append(&model_entry);

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
                let model_c = model_entry.clone();
                let prompt_c = prompt_entry.clone();
                let refresh_dropdown_save = refresh_dropdown.clone();
                save_btn.connect_clicked(move |_| {
                    let name = name_c.text().to_string();
                    let desc = desc_c.text().to_string();
                    let model = model_c.text().to_string();
                    let prompt = prompt_c.text().to_string();
                    
                    {
                        let mut s = state_c.lock().unwrap();
                        if let Some(a) = s.settings.agents.get_mut(idx) {
                            a.name = name;
                            a.description = desc;
                            a.model = model;
                            a.system_prompt = prompt;
                            let _ = fs::write("settings.json", serde_json::to_string(&s.settings).unwrap());
                        }
                    }
                    let refresh_clone = refresh_dropdown_save.clone();
                    glib::idle_add_local(move || {
                        refresh_clone();
                        glib::ControlFlow::Break
                    });
                });

                let state_d = state.clone();
                // We use a simplified deletion that doesn't trigger a full refresh of the same list recursively if possible
                // or we just accept that we need to handle the state carefully.
                delete_btn.connect_clicked(move |_| {
                    let mut s = state_d.lock().unwrap();
                    if idx < s.settings.agents.len() {
                        s.settings.agents.remove(idx);
                        let _ = fs::write("settings.json", serde_json::to_string(&s.settings).unwrap());
                    }
                });
                agents_list.append(&row);
            }
        }
    };

    refresh_agents_list();

    let add_agent_btn = Button::with_label("Add Agent");
    let state_add = state.clone();
    let refresh_agents_list_add = refresh_agents_list.clone();
    add_agent_btn.connect_clicked(move |_| {
        let mut s = state_add.lock().unwrap();
        s.settings.agents.push(Agent {
            name: "New Agent".to_string(),
            model: "llama3".to_string(),
            system_prompt: "You are a helpful assistant.".to_string(),
            description: "Personal Assistant".to_string(),
        });
        let _ = fs::write("settings.json", serde_json::to_string(&s.settings).unwrap());
        drop(s);
        refresh_agents_list_add();
    });

    let delete_chat_history_btn = Button::with_label("Delete Chat History");
    let state_delete_history = state.clone();
    delete_chat_history_btn.connect_clicked(move |_| {
        let mut s = state_delete_history.lock().unwrap();
        s.history.clear();
        let _ = fs::remove_file("history.json");
    });
    general_box.append(&delete_chat_history_btn);
    agents_box.append(&add_agent_btn);
    settings_stack.add_titled(&agents_box, Some("agents"), "Agents");

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

    personalization_box.append(&Label::new(Some("Bubble Color")));
    let bubble_color_entry = Entry::builder()
        .placeholder_text("#0b93f6")
        .build();
    personalization_box.append(&bubble_color_entry);

    let apply_theme_btn = Button::with_label("Apply (Not implemented)");
    personalization_box.append(&apply_theme_btn);

    personalization_box.append(&gtk::Separator::new(Orientation::Horizontal));
    
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
    personalization_box.append(&scrolled_profiles);

    let refresh_profiles_list: Rc<RefCell<Option<std::boxed::Box<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    
    let refresh_profiles_list_impl = {
        let state = state.clone();
        let profiles_list = profiles_scrolled_content.clone();
        let refresh_profiles_list_ref = refresh_profiles_list.clone();
        move || {
            while let Some(child) = profiles_list.first_child() {
                profiles_list.remove(&child);
            }
            let profiles = {
                let s = match state.try_lock() {
                    Ok(s) => s,
                    Err(_) => {
                        eprintln!("Deadlock prevented in refresh_profiles_list");
                        return;
                    }
                };
                s.settings.profiles.clone()
            };
            for (idx, profile) in profiles.into_iter().enumerate() {
                let profile_container = Box::builder()
                    .orientation(Orientation::Vertical)
                    .spacing(5)
                    .build();

                let circle_btn = Button::builder()
                    .css_classes(["profile-circle"])
                    .width_request(80)
                    .height_request(80)
                    .build();
                circle_btn.set_hexpand(false);
                circle_btn.set_vexpand(false);
                
                let icon_label = Label::new(Some(&profile.name.chars().next().unwrap_or('?').to_string().to_uppercase()));
                circle_btn.set_child(Some(&icon_label));

                let name_label = Label::builder()
                    .label(&profile.name)
                    .css_classes(["profile-mini-name"])
                    .halign(gtk::Align::Center)
                    .build();

                profile_container.append(&circle_btn);
                profile_container.append(&name_label);
                profiles_list.append(&profile_container);

                let popover = gtk::Popover::new();
                popover.set_position(gtk::PositionType::Bottom);
                popover.set_autohide(true);
                popover.set_has_arrow(true);
                popover.set_parent(&profile_container);
                popover.set_cascade_popdown(true);

                let popover_box = Box::builder()
                    .orientation(Orientation::Vertical)
                    .spacing(10)
                    .margin_start(15)
                    .margin_end(15)
                    .margin_top(15)
                    .margin_bottom(15)
                    .width_request(300)
                    .build();
                popover.set_child(Some(&popover_box));

                popover_box.append(&Label::builder().label("Edit Profile").css_classes(["settings-title"]).xalign(0.0).build());

                popover_box.append(&Label::builder().label("Profile Name").xalign(0.0).css_classes(["settings-label"]).build());
                let name_entry = Entry::builder().text(&profile.name).placeholder_text("Work, Personal, etc.").build();
                popover_box.append(&name_entry);

                let contact_grid = gtk::Grid::builder()
                    .column_spacing(10)
                    .row_spacing(5)
                    .build();

                let f_name = Entry::builder().text(&profile.first_name).placeholder_text("First Name").hexpand(true).build();
                let l_name = Entry::builder().text(&profile.last_name).placeholder_text("Last Name").hexpand(true).build();
                contact_grid.attach(&f_name, 0, 0, 1, 1);
                contact_grid.attach(&l_name, 1, 0, 1, 1);

                let email = Entry::builder().text(&profile.email).placeholder_text("Email").hexpand(true).build();
                let phone = Entry::builder().text(&profile.phone).placeholder_text("Phone").hexpand(true).build();
                contact_grid.attach(&email, 0, 1, 1, 1);
                contact_grid.attach(&phone, 1, 1, 1, 1);

                popover_box.append(&contact_grid);

                popover_box.append(&Label::builder().label("Location").xalign(0.0).css_classes(["settings-label"]).build());
                let loc_entry = Entry::builder().text(&profile.location).placeholder_text("City, Country").build();
                popover_box.append(&loc_entry);

                popover_box.append(&Label::builder().label("Bio / Context").xalign(0.0).css_classes(["settings-label"]).build());
                let bio_entry = Entry::builder().text(&profile.bio).placeholder_text("Short bio to help LLM know you").build();
                popover_box.append(&bio_entry);

                let actions_box = Box::builder().orientation(Orientation::Horizontal).spacing(10).margin_top(5).build();
                let save_btn = Button::with_label("Save");
                let delete_btn = Button::with_label("Delete");
                actions_box.append(&save_btn);
                actions_box.append(&delete_btn);
                popover_box.append(&actions_box);

                circle_btn.connect_clicked(glib::clone!(#[weak] popover, move |btn| {
                    // Use `get_allocation()` to get the current size and position of the button
                    let allocation = btn.allocation();
                    popover.set_pointing_to(Some(&allocation));
                    popover.popup();
                }));

                let state_c = state.clone();
                let name_c = name_entry.clone();
                let f_name_c = f_name.clone();
                let l_name_c = l_name.clone();
                let email_c = email.clone();
                let phone_c = phone.clone();
                let loc_c = loc_entry.clone();
                let bio_c = bio_entry.clone();
                let refresh_ref_save = refresh_profiles_list_ref.clone();
                
                save_btn.connect_clicked(move |_| {
                    {
                        let mut s = state_c.lock().unwrap();
                        if let Some(p) = s.settings.profiles.get_mut(idx) {
                            p.name = name_c.text().to_string();
                            p.first_name = f_name_c.text().to_string();
                            p.last_name = l_name_c.text().to_string();
                            p.email = email_c.text().to_string();
                            p.phone = phone_c.text().to_string();
                            p.location = loc_c.text().to_string();
                            p.bio = bio_c.text().to_string();
                            let _ = fs::write("settings.json", serde_json::to_string(&s.settings).unwrap());
                        }
                    }
                    if let Some(f) = &*refresh_ref_save.borrow() {
                        f();
                    }
                });

                let state_d = state.clone();
                let refresh_ref_del = refresh_profiles_list_ref.clone();
                delete_btn.connect_clicked(move |_| {
                    {
                        let mut s = state_d.lock().unwrap();
                        if idx < s.settings.profiles.len() {
                            s.settings.profiles.remove(idx);
                            let _ = fs::write("settings.json", serde_json::to_string(&s.settings).unwrap());
                        }
                    }
                    if let Some(f) = &*refresh_ref_del.borrow() {
                        f();
                    }
                });
            }
        }
    };
    
    *refresh_profiles_list.borrow_mut() = Some(std::boxed::Box::new(refresh_profiles_list_impl));

    let add_profile_btn = Button::with_label("Add Profile");
    let state_add_p = state.clone();
    let refresh_ref = refresh_profiles_list.clone();
    add_profile_btn.connect_clicked(move |_| {
        {
            let mut s = state_add_p.lock().unwrap();
            s.settings.profiles.push(Profile {
                name: "New Profile".to_string(),
                first_name: "".to_string(),
                last_name: "".to_string(),
                email: "".to_string(),
                phone: "".to_string(),
                location: "".to_string(),
                bio: "".to_string(),
                image_path: None,
            });
            let _ = fs::write("settings.json", serde_json::to_string(&s.settings).unwrap());
        }
        if let Some(f) = &*refresh_ref.borrow() {
            f();
        }
    });
    personalization_box.append(&add_profile_btn);

    if let Some(f) = &*refresh_profiles_list.borrow() {
        f();
    }

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
        .child(&main_stack)
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
    let refresh_history = {
        let state = state.clone();
        let history_list = history_list.clone();
        let chat_box = chat_box.clone();
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
                let chat_box_h = chat_box.clone();
                let item_messages = item.messages.clone();
                row_btn.connect_clicked(move |_| {
                    let mut s = state_h.lock().unwrap();
                    s.messages = item_messages.clone();
                    
                    // Clear and rebuild chat UI
                    while let Some(child) = chat_box_h.first_child() {
                        chat_box_h.remove(&child);
                    }
                    
                    for msg in &s.messages {
                        if msg.role == ollama_rs::generation::chat::MessageRole::System { continue; }
                        let role_label = if msg.role == ollama_rs::generation::chat::MessageRole::User { "You" } else { "Ollama" };
                        let css_class = if msg.role == ollama_rs::generation::chat::MessageRole::User { "user-message" } else { "bot-message" };
                        
                        let label = Label::builder()
                            .xalign(0.0)
                            .wrap(true)
                            .css_classes([css_class])
                            .build();
                        if msg.role == ollama_rs::generation::chat::MessageRole::User {
                            label.set_halign(gtk::Align::End);
                            label.set_markup(&markdown_to_pango(&msg.content));
                        } else {
                            label.set_markup(&format!("<b>{}:</b> {}", role_label, markdown_to_pango(&msg.content)));
                        }
                        chat_box_h.append(&label);
                    }
                });
                history_list.append(&row_btn);
            }
        }
    };
    refresh_history();

    new_chat_btn.connect_clicked({
        let state = state.clone();
        let chat_box = chat_box.clone();
        move |_| {
            let mut s = state.lock().unwrap();
            s.messages.clear();
            while let Some(child) = chat_box.first_child() {
                chat_box.remove(&child);
            }
        }
    });

    // --- Event Handlers ---
    let state_clone = state.clone();
    agent_dropdown.connect_selected_notify(move |dd| {
        let mut s = state_clone.lock().unwrap();
        s.current_agent_idx = dd.selected() as usize;
        s.messages.clear();
    });

    let state_clone = state.clone();
    let chat_box_clone = chat_box.clone();
    let refresh_history_clone = refresh_history.clone();
    entry.connect_activate(move |entry| {
        let text = entry.text().to_string();
        if text.is_empty() { return; }
        entry.set_text("");

        // Add user message to UI
        let user_label = Label::builder()
            .xalign(0.0)
            .wrap(true)
            .css_classes(["user-message"])
            .halign(gtk::Align::End)
            .build();
        user_label.set_markup(&glib::markup_escape_text(&text));
        chat_box_clone.append(&user_label);

        let state = state_clone.clone();
        let chat_box = chat_box_clone.clone();
        let refresh_history = refresh_history_clone.clone();
        
        // Response label
        let bot_label = Label::builder()
            .label("Thinking...")
            .xalign(0.0)
            .wrap(true)
            .css_classes(["bot-message"])
            .build();
        chat_box.append(&bot_label);

        glib::MainContext::default().spawn_local(async move {
            let (ollama, model, messages) = {
                let mut s = state.lock().unwrap();
                let agent = s.settings.agents.get(s.current_agent_idx).cloned().unwrap_or_else(|| s.settings.agents[0].clone());
                
                if s.messages.is_empty() {
                    s.messages.push(ChatMessage::system(agent.system_prompt.clone()));
                }
                
                s.messages.push(ChatMessage::user(text.clone()));
                (s.ollama.clone(), agent.model.clone(), s.messages.clone())
            };

            match ollama.send_chat_messages_stream(
                ChatMessageRequest::new(model.clone(), messages)
            ).await {
                Ok(mut stream) => {
                    let mut full_response = String::new();
                    while let Some(res) = stream.next().await {
                        if let Ok(res) = res {
                            let msg = res.message;
                            full_response.push_str(&msg.content);
                            bot_label.set_markup(&markdown_to_pango(&full_response));
                        }
                    }
                    
                    let is_first_message;
                    let history_id = glib::uuid_string_random().to_string();
                    
                    {
                        let mut s = state.lock().unwrap();
                        s.messages.push(ChatMessage::assistant(full_response));
                        is_first_message = s.messages.len() <= 3;
                        
                        let history_item = ChatHistory {
                            id: history_id.clone(),
                            title: text.chars().take(20).collect(),
                            messages: s.messages.clone(),
                        };
                        s.history.push(history_item);
                        let _ = fs::write("history.json", serde_json::to_string(&s.history).unwrap());
                    }
                    
                    if is_first_message {
                        let ollama_c = ollama.clone();
                        let state_c = state.clone();
                        let user_text = text.clone();
                        let model_c = model.clone();
                        
                        tokio::spawn(async move {
                            let title_prompt = format!(
                                "Generate a very short, creative 2-4 word title for a chat that starts with: \"{}\". Output ONLY the title, no quotes or punctuation.",
                                user_text
                            );
                            let req = ChatMessageRequest::new(
                                model_c,
                                vec![ChatMessage::user(title_prompt)]
                            );
                            
                            if let Ok(res) = ollama_c.send_chat_messages(req).await {
                                let new_title = res.message.content.trim().trim_matches('"').trim_matches('.').to_string();
                                if !new_title.is_empty() {
                                    let mut s = state_c.lock().unwrap();
                                    if let Some(hist) = s.history.iter_mut().find(|h| h.id == history_id) {
                                        hist.title = new_title;
                                        let _ = fs::write("history.json", serde_json::to_string(&s.history).unwrap());
                                    }
                                }
                            }
                        });
                    }

                    glib::idle_add_local(move || {
                        refresh_history();
                        glib::ControlFlow::Break
                    });
                }
                Err(e) => {
                    bot_label.set_label(&format!("Error: {:?}", e));
                }
            }
        });
    });

    // Load CSS
    let provider = gtk::CssProvider::new();
    provider.load_from_data("
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
    ");
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().expect("Could not connect to a display."),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    window.present();
}
