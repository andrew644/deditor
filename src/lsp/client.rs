use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};

const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;

pub enum RpcMsg {
    Response {
        id: u64,
        /// Absent when the request failed; handlers treat it like a null
        /// result (no hover / no definition / empty completion).
        result: Option<Value>,
    },
    Notification {
        method: String,
        params: Value,
    },
    ServerRequest {
        id: Value,
        method: String,
    },
}

enum WriteReq {
    Notify(String, Value),
    Request(u64, String, Value),
    Respond(Value),
    Shutdown,
}

pub struct LspClient {
    wtx: Sender<WriteReq>,
    pub rx: Receiver<RpcMsg>,
    child: Option<Child>,
    pub next_id: u64,
}

impl LspClient {
    pub fn spawn(bin: &str, args: &[&str], root: &str) -> Result<Self, String> {
        let mut cmd = Command::new(bin);
        cmd.args(args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(unix)]
        unsafe {
            use std::os::unix::process::CommandExt;
            cmd.pre_exec(|| {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("failed to start {bin}: {e}"))?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let (wtx, wrx) = channel::<WriteReq>();
        let (rtx, rx) = channel::<RpcMsg>();
        std::thread::spawn(move || {
            let mut stdin = stdin;
            for req in wrx.iter() {
                let msg = match req {
                    WriteReq::Notify(method, params) => serde_json::json!({
                        "jsonrpc": "2.0", "method": method, "params": params
                    }),
                    WriteReq::Request(id, method, params) => serde_json::json!({
                        "jsonrpc": "2.0", "id": id, "method": method, "params": params
                    }),
                    WriteReq::Respond(response) => response,
                    WriteReq::Shutdown => break,
                };
                let body = msg.to_string();
                if Self::write_frame(&mut stdin, &body).is_err() {
                    break;
                }
            }
        });
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut len: Option<usize> = None;
                loop {
                    let mut line = String::new();
                    match reader.read_line(&mut line) {
                        Ok(0) => return,
                        Ok(_) => {
                            let t = line.trim_end();
                            if t.is_empty() {
                                break;
                            }
                            if let Some(v) = t
                                .strip_prefix("Content-Length:")
                                .or_else(|| t.strip_prefix("content-length:"))
                                .and_then(|v| v.trim().parse::<usize>().ok())
                            {
                                len = Some(v);
                            }
                        }
                        Err(_) => return,
                    }
                }
                let Some(n) = len else { continue };
                if n > MAX_FRAME_BYTES {
                    return;
                }
                let mut buf = vec![0u8; n];
                if reader.read_exact(&mut buf).is_err() {
                    return;
                }
                let Ok(val) = serde_json::from_slice::<Value>(&buf) else {
                    continue;
                };
                let method = val.get("method").and_then(|m| m.as_str());
                let id = val.get("id").cloned();
                match (id, method) {
                    (Some(id), Some(method)) => {
                        let _ = rtx.send(RpcMsg::ServerRequest {
                            id,
                            method: method.to_string(),
                        });
                    }
                    (None, Some(method)) => {
                        let _ = rtx.send(RpcMsg::Notification {
                            method: method.to_string(),
                            params: val.get("params").cloned().unwrap_or(Value::Null),
                        });
                    }
                    (Some(Value::Number(id)), None) => {
                        let _ = rtx.send(RpcMsg::Response {
                            id: id.as_u64().unwrap_or(0),
                            result: val.get("result").cloned(),
                        });
                    }
                    _ => {}
                }
            }
        });
        Ok(Self {
            wtx,
            rx,
            child: Some(child),
            next_id: 1,
        })
    }

    fn write_frame(stdin: &mut impl Write, body: &str) -> std::io::Result<()> {
        write!(stdin, "Content-Length: {}\r\n\r\n{}", body.len(), body)?;
        stdin.flush()
    }

    pub fn notify(&self, method: &str, params: Value) {
        let _ = self.wtx.send(WriteReq::Notify(method.to_string(), params));
    }

    pub fn request(&mut self, method: &str, params: Value) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let _ = self
            .wtx
            .send(WriteReq::Request(id, method.to_string(), params));
        id
    }

    pub fn respond_null(&self, id: Value) {
        self.respond(id, Value::Null)
    }

    pub fn respond(&self, id: Value, result: Value) {
        let _ = self.wtx.send(WriteReq::Respond(serde_json::json!({
            "jsonrpc": "2.0", "id": id, "result": result
        })));
    }

    pub fn shutdown(&mut self) {
        let _ = self.wtx.send(WriteReq::Request(
            self.next_id,
            "shutdown".into(),
            Value::Null,
        ));
        self.next_id += 1;
        let _ = self.wtx.send(WriteReq::Notify("exit".into(), Value::Null));
        let _ = self.wtx.send(WriteReq::Shutdown);
        if let Some(c) = &mut self.child {
            let _ = c.kill();
            let _ = c.wait();
        }
        self.child = None;
    }
}

impl Drop for LspClient {
    fn drop(&mut self) {
        self.shutdown();
    }
}
