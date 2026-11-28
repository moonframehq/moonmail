//! Renderer inputs are either escaped plain text or sanitized HTML. Only the
//! selected backend prepares a document, and no body crosses the Luau boundary.
use {
    crate::{fonts::ReaderFont, native_reader::NativeDocument, page},
    serde::{Deserialize, Serialize},
    std::{
        convert::Infallible,
        io,
        path::Path,
        sync::{Arc, Mutex},
    },
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Renderer {
    Native,
    #[default]
    Webkit,
}

/// Prepare one alternative for display. Implementations consume the same safe
/// fragment and font. The returned value owns everything needed for display.
pub trait ReaderBackend {
    type Output;
    type Error: std::error::Error;
    fn prepare(&self, fragment: &str, font: &ReaderFont) -> Result<Self::Output, Self::Error>;
    fn plain(&self, source: &str, font: &ReaderFont) -> Result<Self::Output, Self::Error> {
        self.prepare(&page::plain_fragment(source), font)
    }
}

pub struct Native;
impl ReaderBackend for Native {
    type Output = NativeDocument;
    type Error = Infallible;
    fn plain(&self, source: &str, font: &ReaderFont) -> Result<Self::Output, Self::Error> {
        Ok(NativeDocument::plain(source, font))
    }
    fn prepare(&self, fragment: &str, font: &ReaderFont) -> Result<Self::Output, Self::Error> {
        Ok(NativeDocument::parse(fragment, font))
    }
}

pub struct WebKit<'a> {
    pub directory: &'a Path,
    pub alternative: &'static str,
    pub revision: u64,
}
impl ReaderBackend for WebKit<'_> {
    type Output = String;
    type Error = io::Error;
    fn prepare(&self, fragment: &str, font: &ReaderFont) -> io::Result<String> {
        let path = self
            .directory
            .join(format!("{}-{}.html", self.alternative, self.revision % 2));
        std::fs::write(&path, page::document(fragment, font))?;
        let mut url = url::Url::from_file_path(path)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Invalid reader path"))?;
        url.set_query(Some(&format!("revision={}", self.revision)));
        Ok(url.into())
    }
}

#[derive(Default)]
pub struct NativePages {
    pub revision: u64,
    pub plain: Option<Arc<NativeDocument>>,
    pub rich: Option<Arc<NativeDocument>>,
}

#[derive(Clone, Default)]
pub struct DocumentSlot(Arc<Mutex<NativePages>>);
impl DocumentSlot {
    pub fn replace(&self, pages: NativePages) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = pages;
    }

    pub fn read(&self, revision: u64, rich: bool) -> Option<Arc<NativeDocument>> {
        let pages = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if pages.revision != revision {
            return None;
        }
        if rich && let Some(document) = &pages.rich {
            return Some(document.clone());
        }
        pages.plain.clone()
    }
}
