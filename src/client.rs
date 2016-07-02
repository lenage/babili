use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use std::fmt;
// use std::sync::mpsc;

use mio::*;
use mio::tcp::*;
use http_muncher::Parser;
use sha1::Sha1;
use rustc_serialize::base64::{ToBase64, STANDARD};

use http::HttpParser;
use frame::{WebSocketFrame};

const WEBSOCKET_KEY: &'static [u8] = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

fn gen_key(key: &String) -> String {
    let mut m = Sha1::new();
    let mut buf = [0u8; 20];
    m.update(key.as_bytes());
    m.update(WEBSOCKET_KEY);
    m.output(&mut buf);
    buf.to_base64(STANDARD)
}

enum ClientState {
    AwaitingHandshake(Mutex<Parser<HttpParser>>),
    HandshakeResponse,
    Connected
}

pub struct WebSocketClient {
    pub socket: TcpStream,
    headers: Arc<Mutex<HashMap<String, String>>>,
    pub interest: EventSet,
    state: ClientState,
    outgoing: Vec<WebSocketFrame>
}

impl WebSocketClient {
    pub fn read(&mut self) {
        match self.state {
            ClientState::AwaitingHandshake(_) => self.read_handshake(),
            ClientState::Connected => self.read_frame(),
            _ => {}
        }
    }

    fn read_handshake(&mut self) {
        loop {
            let mut buf = [0; 2048];
            match self.socket.try_read(&mut buf) {
                Err(e) => {
                    println!("Error while reading socket: {:?}", e);
                    return
                },
                Ok(None) =>
                    break,
                Ok(Some(_len)) => {
                    let is_upgrade = if let ClientState::AwaitingHandshake(ref parser_state) = self.state {
                        let mut parser = parser_state.lock().unwrap();
                        parser.parse(&buf);
                        parser.is_upgrade()
                    } else { false };

                    if is_upgrade {
                        self.state = ClientState::HandshakeResponse;
                        self.interest.remove(EventSet::readable());
                        self.interest.insert(EventSet::writable());
                        break;
                    }
                }
            }
        }
    }

    fn read_frame(&mut self) {
        let frame = WebSocketFrame::read(&mut self.socket);
        match frame {
            Ok(frame) => {
                println!("{:?}", frame);

                // Add a reply frame to the queue:
                let reply_frame = WebSocketFrame::from("Hi there!");
                self.outgoing.push(reply_frame);

                // Switch the event subscription to the write mode if the queue is not empty:
                if self.outgoing.len() > 0 {
                    self.interest.remove(EventSet::readable());
                    self.interest.insert(EventSet::writable());
                }
            },
            Err(e) => println!("error while reading frame: {}", e)
        }
    }

    pub fn write(&mut self) {
        match self.state {
            ClientState::HandshakeResponse => self.write_handshake(),
            ClientState::Connected => self.write_frames(),
            _ => {}
        }
    }

    fn write_frames(&mut self) {
        println!("sending {} frames", self.outgoing.len());

        for frame in self.outgoing.iter() {
            if let Err(e) = frame.write(&mut self.socket) {
                println!("error on write: {}", e);
            }
        }

        self.outgoing.clear();

        self.interest.remove(EventSet::writable());
        self.interest.insert(EventSet::readable());
    }

    fn write_handshake(&mut self) {
        let headers = self.headers.lock().unwrap();
        let response_key = gen_key(&headers.get("Sec-WebSocket-Key").unwrap());
        let response = fmt::format(format_args!("HTTP/1.1 101 Switching Protocols\r\n\
                                                 Connection: Upgrade\r\n\
                                                 Sec-WebSocket-Accept: {}\r\n\
                                                 Upgrade: websocket\r\n\r\n", response_key));
        self.socket.try_write(response.as_bytes()).unwrap();
        self.state = ClientState::Connected;
        self.interest.remove(EventSet::writable());
        self.interest.insert(EventSet::readable());
    }


    pub fn new(socket: TcpStream) -> WebSocketClient {
        let headers = Arc::new(Mutex::new(HashMap::new()));
        WebSocketClient {
            socket: socket,
            headers: headers.clone(),
            interest: EventSet::readable(),
            outgoing: Vec::new(),
            state: ClientState::AwaitingHandshake(Mutex::new(Parser::request(HttpParser {
                current_key: None,
                headers: headers.clone()
            }))),
        }
    }
}
