use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StreamKind { Home, Cloud }

impl StreamKind {
    pub fn as_path(self) -> &'static str {
        match self { Self::Home => "home", Self::Cloud => "cloud" }
    }
}
