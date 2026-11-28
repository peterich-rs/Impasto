//! AppKit window defaults and bundle metadata.

use anyhow::Context;
use objc2_foundation::{NSBundle, NSNumber, NSString};

pub(crate) struct WindowOptions {
    pub width: f64,
    pub height: f64,
    pub min_width: Option<f64>,
    pub min_height: Option<f64>,
    pub title: String,
    pub autosave_name: Option<String>,
    pub size_overridden: bool,
}

impl WindowOptions {
    pub(crate) fn bundled() -> Self {
        let bundle = NSBundle::mainBundle();
        let number = |key: &str| {
            let value = bundle.objectForInfoDictionaryKey(&NSString::from_str(key))?;
            let size = value.downcast::<NSNumber>().ok()?.doubleValue();
            valid(size).then_some(size)
        };
        let string = |key: &str| {
            bundle
                .objectForInfoDictionaryKey(&NSString::from_str(key))?
                .downcast::<NSString>()
                .ok()
                .map(|value| value.to_string())
        };
        Self {
            width: number("ImpastoWindowWidth").unwrap_or(360.0),
            height: number("ImpastoWindowHeight").unwrap_or(640.0),
            min_width: number("ImpastoWindowMinWidth"),
            min_height: number("ImpastoWindowMinHeight"),
            title: string("ImpastoWindowTitle")
                .or_else(|| string("CFBundleDisplayName"))
                .or_else(|| string("CFBundleName"))
                .unwrap_or_else(|| "Impasto".to_owned()),
            autosave_name: string("ImpastoWindowAutosaveName"),
            size_overridden: false,
        }
    }

    pub(crate) fn set_dimension(
        &mut self,
        flag: &str,
        value: Option<String>,
    ) -> anyhow::Result<()> {
        let value = value.with_context(|| format!("{flag} requires a size in points"))?;
        let size = value
            .parse::<f64>()
            .with_context(|| format!("invalid {flag}"))?;
        anyhow::ensure!(valid(size), "{flag} must be between 200 and 8192 points");
        match flag {
            "--width" => {
                self.width = size;
                self.size_overridden = true;
            }
            "--height" => {
                self.height = size;
                self.size_overridden = true;
            }
            "--min-width" => self.min_width = Some(size),
            "--min-height" => self.min_height = Some(size),
            _ => anyhow::bail!("unknown window dimension {flag}"),
        }
        Ok(())
    }

    pub(crate) fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.min_width.is_none_or(|size| size <= self.width)
                && self.min_height.is_none_or(|size| size <= self.height),
            "window minimum exceeds the initial content size"
        );
        Ok(())
    }
}

fn valid(size: f64) -> bool {
    (200.0..=8192.0).contains(&size)
}
