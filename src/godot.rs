use std::collections::HashMap;

use futures::executor;
use gdnative::prelude::*;
use tokio::{
    runtime::{Builder, Runtime},
    sync::mpsc,
    task::LocalSet,
};

use crate::{
    client::{
        ArchipelagoClient, ArchipelagoClientReceiver, ArchipelagoClientSender, ArchipelagoError,
    },
    protocol::{
        ApValue, Bounce, ClientMessage, ClientStatus, Connect, DataStorageOperation, GameData, Get,
        LocationChecks, LocationScouts, NetworkVersion, RoomInfo, Say, ServerMessage, Set,
        StatusUpdate,
    },
};

fn init(handle: InitHandle) {
    gdnative::tasks::register_runtime(&handle);
    gdnative::tasks::set_executor(EXECUTOR.with(|e| *e));
    handle.add_class::<GodotArchipelagoClient>();
    handle.add_class::<GodotArchipelagoClientFactory>();
    handle.add_class::<AsyncExecutorDriver>();
}

godot_init!(init);

#[derive(NativeClass)]
#[no_constructor]
#[inherit(Node)]
pub struct GodotArchipelagoClient {
    #[property]
    url: String,

    room_info: RoomInfo,

    #[property]
    data_package: HashMap<String, GameData>,

    send_message_queue: mpsc::Sender<ClientMessage>,
    receive_message_queue: mpsc::Receiver<ServerMessage>,
}

#[methods]
impl GodotArchipelagoClient {
    fn enqueue_message(&self, message: ClientMessage) -> bool {
        godot_print!("Sending message {message:?}");
        self.send_message_queue.blocking_send(message).is_ok()
    }

    #[method]
    pub fn get_received_messages(&mut self) -> Vec<Variant> {
        let mut messages: Vec<Variant> = vec![];
        loop {
            match self.receive_message_queue.try_recv() {
                Ok(message) => messages.push(message.to_variant()),
                Err(_err) => break,
            }
        }
        messages
    }

    #[method]
    pub fn connect(
        &self,
        game: String,
        name: String,
        password: Option<String>,
        items_handling: Option<i32>,
        tags: Vec<String>,
    ) -> bool {
        let message = ClientMessage::Connect(Connect {
            game,
            name,
            password,
            items_handling,
            tags,
            uuid: "".to_string(),
            version: NetworkVersion {
                major: 0,
                minor: 4,
                build: 4,
                class: "Version".to_string(),
            },
        });
        godot_print!("Sending Connect message: {message:?}");
        self.enqueue_message(message)
    }

    #[method]
    pub fn say(&self, message: String) -> bool {
        let message = ClientMessage::Say(Say { text: message });
        self.enqueue_message(message)
    }

    #[method]
    pub fn sync(&mut self) -> bool {
        let message = ClientMessage::Sync;
        self.enqueue_message(message)
    }

    #[method]
    pub fn location_checks(&self, locations: Vec<i32>) -> bool {
        let message = ClientMessage::LocationChecks(LocationChecks { locations });
        self.enqueue_message(message)
    }

    #[method]
    pub fn location_scouts(&self, locations: Vec<i32>, create_as_hint: i32) -> bool {
        let message = ClientMessage::LocationScouts(LocationScouts {
            locations,
            create_as_hint,
        });
        self.enqueue_message(message)
    }

    #[method]
    pub fn status_update(&self, status: ClientStatus) -> bool {
        let message = ClientMessage::StatusUpdate(StatusUpdate { status });
        self.enqueue_message(message)
    }

    #[method]
    pub fn bounce(
        &self,
        games: Option<Vec<String>>,
        slots: Option<Vec<String>>,
        tags: Option<Vec<String>>,
        data: ApValue,
    ) -> bool {
        let message = ClientMessage::Bounce(Bounce {
            games,
            slots,
            tags,
            data,
        });
        self.enqueue_message(message)
    }

    #[method]
    pub fn get(&self, keys: Vec<String>) -> bool {
        let message = ClientMessage::Get(Get { keys });
        self.enqueue_message(message)
    }

    #[method]
    pub fn set(
        &self,
        key: String,
        default: Variant,
        want_reply: bool,
        operations: Vec<(String, Variant)>,
    ) -> bool {
        let default_json = serde_json::to_value(default.dispatch()).unwrap();
        let data_storage_operations = operations
            .into_iter()
            .map(|(op, value)| DataStorageOperation {
                replace: op,
                value: ApValue(serde_json::to_value(value.dispatch()).unwrap()),
            })
            .collect();
        let message = ClientMessage::Set(Set {
            key,
            default: ApValue(default_json),
            want_reply,
            operations: data_storage_operations,
        });
        self.enqueue_message(message)
    }

    #[method]
    pub fn room_info(&self) -> Variant {
        self.room_info.to_variant()
    }
}

impl ToVariant for GodotArchipelagoClient {
    fn to_variant(&self) -> Variant {
        self.url.to_variant()
    }
}

#[derive(NativeClass, ToVariant, FromVariant)]
#[inherit(Node)]
struct GodotArchipelagoClientFactory {}

impl GodotArchipelagoClientFactory {
    fn new(_base: &Node) -> Self {
        GodotArchipelagoClientFactory {}
    }
}

#[methods]
impl GodotArchipelagoClientFactory {
    #[method]
    fn _ready(&self, #[base] _base: &Node) {
        godot_print!("*******Hello from the client factory in Rust!!******");
    }

    #[method]
    fn boop(&self) {
        godot_print!("*******BEEP from the client factory in Rust!!******");
    }

    #[method]
    fn create_client(&self, url: String) -> Result<GodotArchipelagoClient, ArchipelagoError> {
        godot_print!("Creating client with url {url:?}");
        let client = executor::block_on(ArchipelagoClient::new(url.as_str()))
            .map_err(|e| godot_error!("Error creating client {e:?}"))
            .unwrap();
        let room_info: RoomInfo = client.room_info().to_owned();
        let data_package: HashMap<String, GameData> = match client.data_package() {
            Some(dp) => dp.games.clone(),
            None => HashMap::new(),
        };

        // Setup send/receive tasks
        let (sender, receiver) = client.split();
        let (send_queue_tx, send_queue_rx) = mpsc::channel::<ClientMessage>(1000);
        let (receive_queue_tx, receive_queue_rx) = mpsc::channel::<ServerMessage>(1000);
        tokio::spawn(async {
            recv_message_task(receiver, receive_queue_tx).await;
        });
        tokio::spawn(async {
            send_messages_task(sender, send_queue_rx).await;
        });

        let client = GodotArchipelagoClient {
            url,
            room_info,
            data_package,
            send_message_queue: send_queue_tx,
            receive_message_queue: receive_queue_rx,
        };
        Ok(client)
    }
}

async fn recv_message_task(
    mut receiver: ArchipelagoClientReceiver,
    queue: mpsc::Sender<ServerMessage>,
) {
    loop {
        if let Ok(message) = receiver.recv().await {
            if let Some(message) = message {
                queue.send(message).await.unwrap();
            }
        } else {
            break;
        }
    }
}

async fn send_messages_task(
    mut sender: ArchipelagoClientSender,
    mut queue: mpsc::Receiver<ClientMessage>,
) {
    loop {
        match queue.recv().await {
            // TODO: handle send error
            Some(message) => sender.send(message).await.unwrap(),
            None => {
                // Shutdown
                queue.close();
                break;
            }
        };
    }
}

#[derive(Default)]
struct SharedLocalPool {
    local_set: LocalSet,
}

thread_local! {
    static EXECUTOR: &'static SharedLocalPool = {
        Box::leak(Box::new(SharedLocalPool::default()))
    };
}

impl futures::task::LocalSpawn for SharedLocalPool {
    fn spawn_local_obj(
        &self,
        future: futures::task::LocalFutureObj<'static, ()>,
    ) -> Result<(), futures::task::SpawnError> {
        self.local_set.spawn_local(future);
        Ok(())
    }
}

#[derive(NativeClass)]
#[inherit(Node)]
struct AsyncExecutorDriver {
    runtime: Runtime,
}

impl AsyncExecutorDriver {
    fn new(_base: &Node) -> Self {
        AsyncExecutorDriver {
            runtime: Builder::new_current_thread()
                .enable_io() // optional, depending on your needs
                .build()
                .unwrap(),
        }
    }
}

#[methods]
impl AsyncExecutorDriver {
    #[method]
    fn _process(&self, #[base] _base: &Node, _delta: f64) {
        EXECUTOR.with(|e| {
            self.runtime
                .block_on(async {
                    e.local_set
                        .run_until(async { tokio::task::spawn_local(async {}).await })
                        .await
                })
                .unwrap()
        })
    }
}
