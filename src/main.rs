use clap::Parser;
use serde::{de, Deserialize, Serialize};
use std::{
    collections::HashMap,
    io::{self, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream, UdpSocket},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};
use thiserror::Error;

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Vec3 {
    x: f32,
    y: f32,
    z: f32,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Rot3 {
    x: i16,
    y: i16,
    z: i16,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClientRequest {
    version: Version,
    //udp_port: u16,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Version {
    major: u8,
    minor: u8,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ClientResponse {
    id: usize,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ClientAnimationData {
    id: u32,
    frame: u32,
    frame_alt: u32,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ClientData {
    player_sub_type: u32,
    pos: Vec3,
    rot: Rot3,
    animation: ClientAnimationData,
}

#[derive(Parser)]
struct Args {
    #[arg(short, long, default_value = "0.0.0.0:46944")]
    bind_address: String,
}

#[derive(Error, Debug)]
enum DataError {
    #[error("IO Error")]
    IoError(io::Error),
    #[error("Error deserializing Json")]
    JsonError(serde_json::Error),
}

fn recv<T: de::DeserializeOwned>(stream: &mut TcpStream) -> Result<T, DataError> {
    // recv length as u16 in be
    let mut buffer = [0; 2];
    stream
        .read(&mut buffer)
        .map_err(|e| DataError::IoError(e))?;
    let length = u16::from_be_bytes(buffer);

    // recv serialized data
    let mut buffer = vec![0; length as usize];
    stream
        .read_exact(&mut buffer)
        .map_err(|e| DataError::IoError(e))?;

    // deserialize data
    serde_json::from_slice(&buffer).map_err(|e| DataError::JsonError(e))
}

fn recv_udp<T: de::DeserializeOwned>(
    socket: &mut UdpSocket,
) -> Result<(T, SocketAddr), DataError> {
    const MAX_DATAGRAM_SIZE: usize = 8 * 1024;

    // receive serialized data
    let mut buffer = vec![0; MAX_DATAGRAM_SIZE];
    let (_, remote) = socket
        .recv_from(&mut buffer)
        .map_err(|e| DataError::IoError(e))?;

    // deserialize data
    Ok((
        serde_json::from_slice(&buffer).map_err(|e| DataError::JsonError(e))?,
        remote,
    ))
}

fn send<T: Serialize>(stream: &mut TcpStream, data: &T) -> io::Result<()> {
    // serialize data
    let serialized = serde_json::to_string(data).unwrap();

    // send length as u16 in be
    let length = serialized.len() as u16;
    stream.write(&length.to_be_bytes())?;

    // send serialized data
    stream.write(serialized.as_bytes())?;
    Ok(())
}

fn send_udp<T: Serialize>(socket: &mut UdpSocket, data: &T) -> io::Result<()> {
    // serialize data
    let serialized = serde_json::to_string(data).unwrap();

    // send serialized data
    socket.send(serialized.as_bytes())?;
    Ok(())
}

fn handle_client(
    stream: &mut TcpStream,
    index: usize,
    clients: &Arc<Mutex<Clients>>,
) -> io::Result<()> {
    // receieve client information (todo: something with version number)
    let client_request = match recv::<ClientRequest>(stream) {
        Ok(client_request) => client_request,
        Err(e) => match e {
            DataError::IoError(e) => return Err(e),
            DataError::JsonError(e) => return Err(io::Error::new(io::ErrorKind::Other, e)),
        },
    };
    println!(
        "client {} version: {}.{}",
        index, client_request.version.major, client_request.version.minor
    );

    // send client id
    send(stream, &ClientResponse { id: index })?;

    // send data (after data is recieved) until the client disconnects
    loop {
        // theoretically support 60 fps, only local could probably do that
        thread::sleep(Duration::from_millis(1000 / 60));

        let data = match recv(stream) {
            Ok(data) => data,
            Err(e) => match e {
                DataError::IoError(e) => return Err(e),
                DataError::JsonError(_) => continue,
            },
        };

        let id_to_data = {
            let mut clients = clients.lock().unwrap();
            // insert into clients
            clients.id_to_data.insert(index, data);
            // serialize clients
            clients.id_to_data.clone()
        };
        send(stream, &id_to_data)?;
    }
}

struct Clients {
    addr_to_id: HashMap<SocketAddr, usize>,
    id_to_data: HashMap<usize, ClientData>,
}

fn main() -> std::io::Result<()> {
    let args = Args::parse();

    let clients = Arc::new(Mutex::new(Clients {
        addr_to_id: HashMap::new(),
        id_to_data: HashMap::new(),
    }));

    let mut index = 0;

    let listener = TcpListener::bind(&args.bind_address)?;
    //let udp_socket = UdpSocket::bind(&args.bind_address)?;

    // // udp thread
    // let clients_udp = clients.clone();
    // thread::spawn(move || loop {
    //     thread::sleep(Duration::from_millis(1000 / 60));
    //     loop {
    //         let (data, remote) = match recv_udp(&mut udp_socket) {
    //             Ok(v) => v,
    //             Err(e) => continue,
    //         };
    //         // let mut clients = client_data_udp.lock().unwrap();
    //         // let mut client_map = client_map.lock().unwrap();
    //     }
    // });

    // accept incoming connections and spawn a thread for each
    for stream in listener.incoming() {
        let mut stream = stream.unwrap();
        let clients = clients.clone();
        {
            clients
                .lock()
                .unwrap()
                .addr_to_id
                .insert(stream.peer_addr().unwrap(), index);
        }

        thread::spawn(move || {
            println!(
                "client {} connected, ip: {}",
                index,
                stream.peer_addr().unwrap()
            );

            handle_client(&mut stream, index, &clients);
            println!("client {} disconnected", index);
            // client disconnected, remove from clients
            let mut clients = clients.lock().unwrap();
            clients.id_to_data.remove(&index);
            clients.addr_to_id.remove(&stream.peer_addr().unwrap());
        });
        index += 1;
    }
    Ok(())
}
