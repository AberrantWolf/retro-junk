use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::FrontendError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MiximageLayout {
    pub canvas: CanvasConfig,
    pub screenshot: ScreenshotConfig,
    #[serde(rename = "box")]
    pub box_art: BoxConfig,
    pub marquee: MarqueeConfig,
    pub physical_media: PhysicalMediaConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanvasConfig {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenshotConfig {
    pub max_width: u32,
    pub max_height: u32,
    pub x_offset: i32,
    pub frame_width: u32,
    pub corner_radius: u32,
    pub frame_color: FrameColor,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoxConfig {
    pub max_width: u32,
    pub max_height: u32,
    pub position: AnchorPosition,
    pub prefer_3d: bool,
    pub shadow: ShadowConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarqueeConfig {
    pub max_width: u32,
    pub max_height: u32,
    pub position: AnchorPosition,
    pub shadow: ShadowConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhysicalMediaConfig {
    pub max_width: u32,
    pub max_height: u32,
    pub position: PhysMediaPosition,
    pub gap: u32,
    pub shadow: ShadowConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowConfig {
    pub enabled: bool,
    pub offset: u32,
    pub opacity: f32,
    pub blur_passes: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FrameColor {
    Auto,
    Fixed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AnchorPosition {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PhysMediaPosition {
    RightOfBox,
    LeftOfBox,
}

impl Default for ShadowConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            offset: 12,
            opacity: 0.6,
            blur_passes: 4,
        }
    }
}

impl Default for MiximageLayout {
    fn default() -> Self {
        Self {
            canvas: CanvasConfig {
                width: 1280,
                height: 960,
            },
            screenshot: ScreenshotConfig {
                max_width: 1060,
                max_height: 800,
                x_offset: 40,
                frame_width: 12,
                corner_radius: 16,
                frame_color: FrameColor::Auto,
            },
            box_art: BoxConfig {
                max_width: 620,
                max_height: 600,
                position: AnchorPosition::BottomLeft,
                prefer_3d: true,
                shadow: ShadowConfig::default(),
            },
            marquee: MarqueeConfig {
                max_width: 620,
                max_height: 460,
                position: AnchorPosition::TopRight,
                shadow: ShadowConfig::default(),
            },
            physical_media: PhysicalMediaConfig {
                max_width: 300,
                max_height: 240,
                position: PhysMediaPosition::RightOfBox,
                gap: 32,
                shadow: ShadowConfig::default(),
            },
        }
    }
}

impl MiximageLayout {
    /// Config file path: `~/.config/retro-junk/miximage-layout.yaml`
    pub fn config_path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("retro-junk").join("miximage-layout.yaml"))
    }

    /// Load from disk if it exists, otherwise write the default and return it.
    pub fn load_or_create() -> Result<Self, FrontendError> {
        let path = Self::config_path().ok_or_else(|| {
            FrontendError::InvalidMetadata("Could not determine config directory".to_string())
        })?;

        if path.exists() {
            Self::load_from(&path)
        } else {
            let layout = Self::default();
            layout.save_to(&path)?;
            Ok(layout)
        }
    }

    /// Load layout from a specific YAML file.
    pub fn load_from(path: &Path) -> Result<Self, FrontendError> {
        let contents = std::fs::read_to_string(path)?;
        serde_yml::from_str(&contents).map_err(|e| {
            FrontendError::InvalidMetadata(format!("Invalid miximage layout YAML: {}", e))
        })
    }

    /// Write the layout to a YAML file.
    pub fn save_to(&self, path: &Path) -> Result<(), FrontendError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let yaml = serde_yml::to_string(self).map_err(|e| {
            FrontendError::InvalidMetadata(format!("Failed to serialize layout: {}", e))
        })?;
        std::fs::write(path, yaml)?;
        Ok(())
    }
}
