use serde::{Serialize, Deserialize};
use ollama_rs::generation::chat::ChatMessage;
use ollama_rs::Ollama;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone)]
pub struct Agent {
    pub name: String,
    pub model: String,
    pub system_prompt: String,
    pub description: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Profile {
    pub name: String,
    pub first_name: String,
    pub last_name: String,
    pub email: String,
    pub phone: String,
    pub location: String,
    pub bio: String,
    pub image_path: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Settings {
    pub ollama_endpoint: String,
    pub agents: Vec<Agent>,
    #[serde(default)]
    pub profiles: Vec<Profile>,
    #[serde(default)]
    pub active_profile: Option<String>,
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
            active_profile: None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ChatHistory {
    pub id: String,
    pub title: String,
    pub messages: Vec<ChatMessage>,
}

pub enum ChatEvent {
    Chunk(String),
    Done(String),
    Error(String),
    RefreshHistory,
}

pub struct AppState {
    pub ollama: Ollama,
    pub current_agent_idx: usize,
    pub messages: Vec<ChatMessage>,
    pub history: Vec<ChatHistory>,
    pub settings: Settings,
    pub config_path: PathBuf,
    pub history_path: PathBuf,
    pub current_task: Option<tokio::task::AbortHandle>,
}
