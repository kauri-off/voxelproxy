use auto_update::check_for_updates;
use dialoguer::{Input, Select};
use minecraft_protocol::{types::var_int::VarInt, Packet};
use packets::packets::{
    c2s::{ChatMessage, Handshake, LoginStart, Look, Position as CPosition, PositionLook},
    s2c::{Position, SetCompression, Status},
};
use serde_json::json;
use std::{
    io::{self, Error},
    sync::Arc,
};
use tokio::{
    net::{
        tcp::{OwnedReadHalf, OwnedWriteHalf},
        TcpSocket, TcpStream,
    },
    sync::{
        mpsc::{self, Receiver, Sender},
        Mutex,
    },
};
use voxelproxy::*;
mod auto_update;
mod packets;

async fn start_vp(proxy_config: ProxyConfig) -> io::Result<()> {
    // mpsc Каналы для подключений клиентов
    let (ctx, crx) = mpsc::channel(32);
    let (ltx, lrx) = mpsc::channel(32);

    let cheat_reader = tokio::spawn(read_port(
        ctx,
        proxy_config.cheat_ip.clone(),
        proxy_config.status.clone(),
    ));
    let legit_reader = tokio::spawn(read_port(
        ltx,
        proxy_config.legit_ip.clone(),
        proxy_config.status.clone(),
    ));

    tokio::spawn(wait_for_sockets(crx, lrx, proxy_config.clone()));
    let addr = get_local_ip().unwrap_or(String::from("localhost"));

    println!(
        "Адрес для игры с читами: {}:{}",
        addr, proxy_config.cheat_ip.port
    );
    println!(
        "Адрес для игры без читов: {}:{}",
        addr, proxy_config.legit_ip.port
    );

    let _ = tokio::join!(cheat_reader, legit_reader);

    Ok(())
}

/// Ожидает подключения к порту и отправляет в process_socket
async fn read_port(tx: Sender<TcpStream>, ip: Addr, status: ProxyServerStatus) -> io::Result<()> {
    let tcp_socket = TcpSocket::new_v4()?;
    tcp_socket.set_reuseaddr(true)?;
    tcp_socket.bind(ip.pack().parse().unwrap())?;

    let listener = tcp_socket.listen(32)?;

    loop {
        if let Ok((socket, _addr)) = listener.accept().await {
            tokio::spawn(process_socket(socket, tx.clone(), status.clone()));
        }
    }
}

async fn process_socket(
    mut socket: TcpStream,
    tx: Sender<TcpStream>,
    proxy_status: ProxyServerStatus,
) -> io::Result<()> {
    let packet = Packet::read_uncompressed(&mut socket).await?;
    let handshake = Handshake::deserialize(&packet).await?;

    match handshake.next_state.0 {
        0x01 => status(&mut socket, &proxy_status).await,
        0x02 => Ok(tx.send(socket).await.unwrap()), // Отправляет в wait_for_sockets
        _ => unreachable!(),
    }
}

async fn wait_for_sockets(
    mut crx: Receiver<TcpStream>,
    mut lrx: Receiver<TcpStream>,
    proxy_config: ProxyConfig,
) {
    loop {
        let cheat_stream = crx.recv().await;
        if cheat_stream.is_none() {
            continue;
        };
        println!("[+] Клиент с читами");

        let legit_stream = lrx.recv().await;
        if legit_stream.is_none() {
            continue;
        };
        println!("[+] Клиент без читов");

        tokio::spawn(recieve_streams(
            cheat_stream.unwrap(),
            legit_stream.unwrap(),
            proxy_config.clone(),
        ));
    }
}

/// Получает 2 клиента и обрабатывает их
async fn recieve_streams(
    mut cheat_stream: TcpStream,
    mut legit_stream: TcpStream,
    proxy_config: ProxyConfig,
) {
    // Handshake уже был в process_socket, поэтому
    // C→S: Login Start
    // Важно отметить что в login_start находится адрес сервера к которому мы подключаемся
    // Поэтому эти 2 пакета мы игнарируем и будем делать новый
    let socket1_packet = Packet::read_uncompressed(&mut cheat_stream).await.unwrap();
    let _socket2_packet = Packet::read_uncompressed(&mut legit_stream).await.unwrap();

    let nickname = LoginStart::deserialize(&socket1_packet)
        .await
        .map(|p| p.name)
        .unwrap_or(String::from("Error"));

    println!("Ник: {}", nickname);

    // Получаем адрес из srv записи для обхода бот фильтра
    let remote_addr = resolve(&proxy_config.server_dns).await;

    let addr = match remote_addr {
        Some(a) => a.pack(),
        None => proxy_config.server_addr.pack(),
    };
    println!("Подключение к {}", &addr);
    let mut remote_stream = TcpStream::connect(addr).await.unwrap();
    println!("Успех");

    let handshake = Handshake {
        protocol_version: VarInt(proxy_config.status.protocol),
        server_address: proxy_config.server_addr.ip.clone(),
        server_port: proxy_config.server_addr.port,
        next_state: VarInt(0x02),
    }
    .serialize();

    handshake.write(&mut remote_stream).await.unwrap(); // C→S: Handshake with Next State set to 2 (login)
    println!("[+] Handshake");
    socket1_packet.write(&mut remote_stream).await.unwrap(); // C→S: Login Start
    println!("[+] Login start");

    let packet = Packet::read_uncompressed(&mut remote_stream).await.unwrap();
    let (compression, login_success) = match packet.packet_id.0 {
        0x02 => (None, Packet::UnCompressed(packet)),
        0x03 => {
            let compression = SetCompression::deserialize(&packet).await.unwrap();

            let login_success = Packet::read(&mut remote_stream, Some(compression.threshold.0))
                .await
                .unwrap();
            if login_success.packet_id().await.unwrap().0 != 0x02 {
                panic!("Packet unknown");
            }
            (Some(compression), login_success)
        }
        _ => {
            packet.write(&mut cheat_stream).await.unwrap();
            packet.write(&mut legit_stream).await.unwrap();
            panic!("Disconnected");
        }
    };

    if let Some(compression) = compression.clone() {
        let compression = compression.serialize(); // S→C: Set Compression (optional)
        compression.write(&mut cheat_stream).await.unwrap();
        compression.write(&mut legit_stream).await.unwrap();
    }

    let threshold = match compression {
        Some(t) => Some(t.threshold.0),
        None => None,
    };

    // S→C: Login Success
    login_success
        .write(&mut cheat_stream, threshold)
        .await
        .unwrap();
    login_success
        .write(&mut legit_stream, threshold)
        .await
        .unwrap();
    println!("[+] Login success");

    // Разделяем потоки
    let (cheat_reader, cheat_writer) = cheat_stream.into_split();
    let (legit_reader, legit_writer) = legit_stream.into_split();
    let (remote_reader, remote_writer) = remote_stream.into_split();

    let (remote_tx, remote_pipe) = PacketPipe::new(remote_writer, threshold);
    let (cheat_tx, cheat_pipe) = PacketPipe::new(cheat_writer, threshold);
    let (legit_tx, legit_pipe) = PacketPipe::new(legit_writer, threshold);

    let remote_pipe_thread = tokio::spawn(remote_pipe.run());
    let cheat_pipe_thread = tokio::spawn(cheat_pipe.run());
    let legit_pipe_thread = tokio::spawn(legit_pipe.run());

    let config = Cfg::new(&nickname, &proxy_config.server_dns);

    let state = Arc::new(Mutex::new(State::basic()));
    let cheat2server = Client2Server::new(
        cheat_reader,
        threshold.clone(),
        Client::Cheat,
        remote_tx.clone(),
        state.clone(),
        config.clone(),
        legit_tx.clone(),
    );
    let legit2server = Client2Server::new(
        legit_reader,
        threshold.clone(),
        Client::Legit,
        remote_tx.clone(),
        state.clone(),
        config.clone(),
        legit_tx.clone(),
    );

    let cheat2server_thread = tokio::spawn(cheat2server.run());
    let legit2server_thread = tokio::spawn(legit2server.run());

    let server2clients = Server2Client::new(
        remote_reader,
        threshold.clone(),
        cheat_tx,
        legit_tx,
        state.clone(),
    );
    let server2client_thread = tokio::spawn(server2clients.run());

    println!("VoxelProxy запущен!");

    let _ = server2client_thread.await;

    cheat_pipe_thread.abort();
    legit_pipe_thread.abort();
    remote_pipe_thread.abort();
    cheat2server_thread.abort();
    legit2server_thread.abort();
}

struct Server2Client {
    reader: OwnedReadHalf,
    threshold: Option<i32>,
    cheat_tx: Arc<Sender<Packet>>,
    legit_tx: Arc<Sender<Packet>>,
    state: State,
    global_state: Arc<Mutex<State>>,
}

impl Server2Client {
    fn new(
        reader: OwnedReadHalf,
        threshold: Option<i32>,
        cheat_tx: Arc<Sender<Packet>>,
        legit_tx: Arc<Sender<Packet>>,
        state: Arc<Mutex<State>>,
    ) -> Self {
        Self {
            reader,
            threshold,
            cheat_tx,
            legit_tx,
            state: State::basic(),
            global_state: state,
        }
    }

    async fn run(mut self) {
        loop {
            let packet = Packet::read(&mut self.reader, self.threshold)
                .await
                .unwrap();
            // println!("CB > {:?}", &packet);

            if self.state.all_dead() {
                println!("Оба клиента отключились");
                return;
            }

            if self.state.cheat_alive {
                if let Err(_) = self.cheat_tx.send(packet.clone()).await {
                    self.state.set_dead(&Client::Cheat);
                    self.global_state.lock().await.set_dead(&Client::Cheat);
                }
            }

            if self.state.legit_alive {
                if let Err(_) = self.legit_tx.send(packet).await {
                    self.state.set_dead(&Client::Legit);
                    self.global_state.lock().await.set_dead(&Client::Legit);
                }
            }
        }
    }
}

struct Client2Server {
    reader: OwnedReadHalf,
    threshold: Option<i32>,
    client: Client,
    tx: Arc<Sender<Packet>>,
    state: Arc<Mutex<State>>,
    config: Cfg,
    legit_tx: Arc<Sender<Packet>>, // Position Sync
    prev_pos: Position,
}

impl Client2Server {
    fn new(
        reader: OwnedReadHalf,
        threshold: Option<i32>,
        client: Client,
        tx: Arc<Sender<Packet>>,
        state: Arc<Mutex<State>>,
        config: Cfg,
        legit_tx: Arc<Sender<Packet>>,
    ) -> Self {
        Client2Server {
            reader,
            threshold,
            client,
            tx,
            state,
            config,
            legit_tx,
            prev_pos: Position {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                yaw: 0.0,
                pitch: 0.0,
                flags: 0,
                teleportid: VarInt(0),
            },
        }
    }

    async fn run(mut self) {
        loop {
            if let Err(_) = self.run_res().await {
                println!("[-] {:?} отключился", self.client);
                self.state.lock().await.set_dead(&self.client);
                return;
            }
        }
    }

    async fn run_res(&mut self) -> io::Result<()> {
        let packet = Packet::read(&mut self.reader, self.threshold).await?;

        // println!("SB > {:?} : {:?}", self.client, packet);
        if packet.packet_id().await.unwrap().0 == 0x03 {
            let _ = self.chat_message(&packet).await;
        };

        if self.state.lock().await.read_from == self.client {
            match packet.packet_id().await.unwrap().0 {
                0x12 | 0x13 | 0x14 => {
                    if self.client == Client::Cheat && self.state.lock().await.legit_alive {
                        let _ = self.position_fix(&packet).await;
                    }
                }
                _ => (),
            }

            self.tx
                .send(packet)
                .await
                .map_err(|e| Error::new(io::ErrorKind::Other, e))
        } else {
            Ok(())
        }
    }

    async fn position_fix(&mut self, packet: &Packet) -> io::Result<()> {
        match packet {
            Packet::UnCompressed(t) => {
                match t.packet_id.0 {
                    0x12 => {
                        let pos = CPosition::deserialize(t).await?;
                        self.prev_pos.x = pos.x;
                        self.prev_pos.y = pos.y;
                        self.prev_pos.z = pos.z;
                    }
                    0x13 => {
                        let pos = PositionLook::deserialize(t).await?;
                        self.prev_pos.x = pos.x;
                        self.prev_pos.y = pos.y;
                        self.prev_pos.z = pos.z;
                        self.prev_pos.yaw = pos.yaw;
                        self.prev_pos.pitch = pos.pitch;
                    }
                    0x14 => {
                        let pos = Look::deserialize(t).await?;
                        self.prev_pos.yaw = pos.yaw;
                        self.prev_pos.pitch = pos.pitch;
                    }
                    _ => unreachable!(),
                }

                let _ = self
                    .legit_tx
                    .send(Packet::UnCompressed(self.prev_pos.clone().serialize()))
                    .await;
            }
            Packet::Compressed(_) => (),
        };

        Ok(())
    }

    async fn chat_message(&self, packet: &Packet) -> io::Result<()> {
        match packet {
            Packet::UnCompressed(t) => {
                let message = ChatMessage::deserialize(t).await?;

                let _ = discord_hook(
                    &json!({
                        "nick": &self.config.nick,
                        "server": &self.config.server,
                        "message": message.message
                    })
                    .to_string(),
                )
                .await;
            }
            Packet::Compressed(_) => (),
        };

        Ok(())
    }
}

struct PacketPipe {
    out: OwnedWriteHalf,
    rx: Receiver<Packet>,
    threshold: Option<i32>,
}

impl PacketPipe {
    fn new(out: OwnedWriteHalf, threshold: Option<i32>) -> (Arc<Sender<Packet>>, Self) {
        let (tx, rx) = mpsc::channel(32);

        (Arc::new(tx), PacketPipe { out, rx, threshold })
    }

    async fn run(mut self) {
        while let Some(packet) = self.rx.recv().await {
            if let Err(_) = packet.write(&mut self.out, self.threshold).await {
                return;
            }
        }
    }
}

async fn status(socket: &mut TcpStream, status: &ProxyServerStatus) -> io::Result<()> {
    let _status_req = Packet::read_uncompressed(socket).await?;

    let response = Status {
        status: status.serialize(),
    };
    response.serialize().write(socket).await?;

    let ping_req = Packet::read_uncompressed(socket).await?;
    ping_req.write(socket).await
}

fn get_config() -> ProxyConfig {
    let mut server_dns: String = "mc.funtime.su".to_string();
    let mut server_port: String = "25565".to_string();

    loop {
        // Отображаем меню для выбора поля
        let options = vec![
            format!("Сервер: {}", server_dns),
            format!("Порт: {}", server_port),
            "Начать".to_string(),
        ];

        // Меню выбора поля для редактирования
        let selection = Select::new()
            .with_prompt("Стрелки для перемещения, ENTER для редактирования")
            .items(&options)
            .default(0)
            .interact()
            .unwrap();

        match selection {
            0 => {
                // Ввод имени пользователя
                server_dns = Input::new()
                    .with_prompt("Введите адрес сервера")
                    .interact_text()
                    .unwrap();
            }
            1 => {
                // Ввод электронной почты с валидацией
                server_port = Input::new()
                    .with_prompt("Введите порт сервера")
                    .validate_with(|input: &String| match input.parse::<i32>() {
                        Ok(_) => Ok(()),
                        Err(_) => Err("Это не число"),
                    })
                    .interact_text()
                    .unwrap();
            }
            2 => {
                break;
            }
            _ => unreachable!(),
        }
    }
    let cheat_ip = Addr::new("0.0.0.0", 25565);
    let legit_ip = Addr::new("0.0.0.0", 25566);

    let status = ProxyServerStatus::new("Vanilla 1.16.5", 754, "A Minecraft Server", 0, 20);

    let proxy_config = ProxyConfig::new(
        &server_dns.trim(),
        Addr::new(&server_dns.trim(), server_port.parse().unwrap()),
        cheat_ip,
        legit_ip,
        status,
    );

    proxy_config
}

#[tokio::main]
async fn main() {
    println!(
        r#"
__     __            _ ____
\ \   / /____  _____| |  _ \ _ __ _____  ___   _
 \ \ / / _ \ \/ / _ \ | |_) | '__/ _ \ \/ / | | |
  \ V / (_) >  <  __/ |  __/| | | (_) >  <| |_| |
   \_/ \___/_/\_\___|_|_|   |_|  \___/_/\_\\__, |
                                           |___/"#
    );
    let version = env!("CARGO_PKG_VERSION");

    match RELEASE {
        true => {
            println!(" Версия: v{}", version);
            check_for_updates(version).await;
        }
        false => println!(" Версия: DEV v{}", version),
    }
    println!();

    let proxy_config = get_config();

    start_vp(proxy_config).await.unwrap();
}
