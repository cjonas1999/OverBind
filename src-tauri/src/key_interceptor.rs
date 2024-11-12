use crate::Settings;

pub(crate) trait KeyInterceptorTrait {
    fn new() -> Self
    where
        Self: Sized;
    fn initialize(&mut self, settings: &Settings) -> Result<(), String>;
    fn start(&mut self, app: &tauri::AppHandle) -> Result<(), String>;
    fn stop(&self, app: &tauri::AppHandle) -> ();
    fn is_running(&self) -> bool;
}
