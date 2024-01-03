use std::collections::HashMap;

use gdnative::prelude::*;
use tokio::sync::mpsc;

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

#[derive(NativeClass)]
#[no_constructor]
#[inherit(Node)]
pub struct GodotArchipelagoClient {
    #[property]
    url: String,

    room_info: RoomInfo,

    // #[property]
    data_package: HashMap<String, GameData>,

    send_message_queue: mpsc::Sender<ClientMessage>,
    receive_message_queue: mpsc::Receiver<ServerMessage>,
}

#[methods]
impl GodotArchipelagoClient {
    fn enqueue_message(&self, message: ClientMessage) -> bool {
        godot_print!("RUST: Sending message {message:?}.");
        match self.send_message_queue.blocking_send(message) {
            Ok(_) => {
                godot_print!("RUST: Sent message successfully.");
                true
            }
            Err(err) => {
                godot_print!("RUST: Failed to send message: {err:?}.");
                false
            }
        }
    }

    #[method]
    pub fn get_received_messages(&mut self) -> Vec<Variant> {
        let mut messages: Vec<Variant> = vec![];
        loop {
            match self.receive_message_queue.try_recv() {
                Ok(message) => {
                    godot_print!("RUST: Received message {message:?}.");
                    messages.push(message.to_variant());
                }
                Err(_err) => break,
            }
        }
        messages
    }

    #[method]
    pub fn connect_to_multiworld(
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

    #[method]
    pub fn data_package(&self) -> Variant {
        self.data_package.to_variant()
    }
}

#[derive(NativeClass)]
#[inherit(Node)]
pub struct GodotArchipelagoClientFactory {
    #[property]
    x: i32,
}

impl GodotArchipelagoClientFactory {
    fn new(_base: &Node) -> Self {
        GodotArchipelagoClientFactory { x: 10 }
    }
}

#[methods]
impl GodotArchipelagoClientFactory {
    #[method]
    fn _ready(&self, #[base] _base: &Node) {}

    #[method(async)]
    fn create_client(
        &self,
        url: String,
    ) -> impl std::future::Future<
        Output = Result<Instance<GodotArchipelagoClient, Shared>, ArchipelagoError>,
    > + 'static {
        godot_print!("Creating client with url {url:?}");
        async move {
            godot_print!("In async block");
            let client = ArchipelagoClient::new(url.as_str())
                .await
                .map_err(|e| godot_error!("Error creating client {e:?}"))
                .unwrap();
            godot_print!("Got client");

            let room_info: RoomInfo = client.room_info().to_owned();
            let data_package: HashMap<String, GameData> = match client.data_package() {
                Some(dp) => dp.games.clone(),
                None => HashMap::new(),
            };
            godot_print!("Got room info and data package");

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
            godot_print!("Spawned send/recv tasks");

            let client = GodotArchipelagoClient {
                url,
                room_info,
                data_package,
                send_message_queue: send_queue_tx,
                receive_message_queue: receive_queue_rx,
            };
            godot_print!("Creating wrapper and returning");
            let node = client.emplace();
            Ok(node.into_shared())
        }
    }
}

async fn recv_message_task(
    mut receiver: ArchipelagoClientReceiver,
    queue: mpsc::Sender<ServerMessage>,
) {
    godot_print!("RUST: Started receive message task.");
    loop {
        match receiver.recv().await {
            Ok(message) => {
                if let Some(message) = message {
                    godot_print!("RUST: Received message.");
                    queue.send(message).await.unwrap();
                } else {
                    godot_print!("RUST: Received empty message.");
                }
            }
            Err(err) => {
                godot_error!("RUST ERROR: Err in receive queue: {err:?}.");
                break;
            }
        }
        // if let Ok(message) = receiver.recv().await {
        //     if let Some(message) = message {
        //         godot_print!("RUST: Received message.");
        //         queue.send(message).await.unwrap();
        //     }
        // } else {
        //     godot_print!("RUST: Shutting down receive queue.");
        //     break;
        // }
    }
}

async fn send_messages_task(
    mut sender: ArchipelagoClientSender,
    mut queue: mpsc::Receiver<ClientMessage>,
) {
    godot_print!("RUST: Started send message task.");
    loop {
        match queue.recv().await {
            // TODO: handle send error
            Some(message) => {
                godot_print!("RUST: Sending message.");
                sender.send(message).await.unwrap()
            }
            None => {
                // Shutdown
                godot_print!("RUST: Shutting down send queue.");
                queue.close();
                break;
            }
        };
    }
}
