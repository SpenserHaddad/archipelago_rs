use gdnative::init::GDNativeCallbacks;

use self::{
    async_executor::{AsyncExecutorDriver, EXECUTOR},
    client::{GodotArchipelagoClient, GodotArchipelagoClientFactory},
};

mod async_executor;
pub mod client;

struct GodotArchipelagoExportLibrary;

#[gdnative::init::callbacks]
impl GDNativeCallbacks for GodotArchipelagoExportLibrary {
    fn nativescript_init(handle: gdnative::init::InitHandle) {
        gdnative::tasks::register_runtime(&handle);
        gdnative::tasks::set_executor(EXECUTOR.with(|e| *e));
        handle.add_class::<GodotArchipelagoClient>();
        handle.add_class::<GodotArchipelagoClientFactory>();
        handle.add_class::<AsyncExecutorDriver>();
    }
}
