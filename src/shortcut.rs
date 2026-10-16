//! Register only while a cue is visible; never poll the keyboard.
use global_hotkey::{
    hotkey::{HotKey, Modifiers},
    GlobalHotKeyManager,
};

pub const DEFAULT: &str = "Ctrl+Alt+X";

pub struct Shortcut {
    manager: Option<GlobalHotKeyManager>,
    key: HotKey,
    registered: bool,
    pub name: String,
    pub error: Option<String>,
}

pub fn parse(value: &str) -> Result<HotKey, String> {
    let key: HotKey = value
        .parse()
        .map_err(|e| format!("Invalid hide shortcut: {e}"))?;
    if !key
        .mods
        .intersects(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER)
    {
        return Err("Hide shortcut needs Ctrl, Alt/Option, or Super/Command".into());
    }
    Ok(key)
}

impl Shortcut {
    pub fn new(name: String) -> Result<Self, String> {
        let key = parse(&name)?;
        Ok(Self {
            // Some backends run a worker while the manager exists, even with no
            // registered keys. Do not create it until a cue is actually visible.
            manager: None,
            key,
            registered: false,
            name,
            error: None,
        })
    }
    pub fn register(&mut self) {
        if self.registered {
            return;
        }
        let manager = match GlobalHotKeyManager::new() {
            Ok(manager) => manager,
            Err(error) => {
                self.error = Some(format!("Global shortcut unavailable: {error}"));
                return;
            }
        };
        match manager.register(self.key) {
            Ok(()) => {
                self.manager = Some(manager);
                self.registered = true;
                self.error = None;
            }
            Err(error) => {
                self.error = Some(format!(
                "Cannot register {} (possibly in use): {error}. Choose --hide-shortcut Ctrl+Alt+H",
                self.name
            ))
            }
        }
    }
    pub fn unregister(&mut self) {
        let was_registered = self.registered;
        self.registered = false;
        if let Some(manager) = self.manager.take() {
            if was_registered {
                if let Err(error) = manager.unregister(self.key) {
                    self.error = Some(format!("Cannot release shortcut: {error}"));
                }
            }
            // Always drop the native manager, including after an unregister
            // error, so no worker or native hotkey window remains while hidden.
            drop(manager);
        }
    }
    pub fn matches(&self, id: u32) -> bool {
        self.registered && self.key.id() == id
    }
    pub fn active(&self) -> bool {
        self.registered
    }
    pub fn id(&self) -> u32 {
        self.key.id()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn configurable_without_intercepting_plain_typing() {
        assert!(parse(DEFAULT).is_ok());
        assert!(parse("Ctrl+Option+H").is_ok());
        assert!(parse("X").is_err());
        assert!(parse("Shift+X").is_err());
        assert!(parse("Ctrl+Alt").is_err());
    }
    #[test]
    fn construction_and_idle_clear_need_no_native_manager() {
        let mut shortcut = Shortcut::new(DEFAULT.to_string()).unwrap();
        assert!(!shortcut.active());
        assert!(shortcut.manager.is_none());
        shortcut.unregister();
        assert!(!shortcut.active());
        assert!(shortcut.manager.is_none());
        assert!(shortcut.error.is_none());
    }
}
