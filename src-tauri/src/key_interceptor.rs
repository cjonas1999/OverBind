use crate::Settings;

pub(crate) trait KeyInterceptorTrait {
    fn new() -> Self
    where
        Self: Sized;
    fn initialize(&mut self, settings: &Settings) -> Result<(), String>;
    fn start(&mut self) -> Result<(), String>;
    fn stop(&self) -> ();
    fn is_running(&self) -> bool;
}
