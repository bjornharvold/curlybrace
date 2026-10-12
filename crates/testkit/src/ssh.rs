//! An sshd container plus a MongoDB reachable only through it.
use crate::container::{MONGO_IMAGE, docker, published_port};
use crate::poll;
use std::time::Duration;

pub const SSH_USER: &str = "tester";
pub const SSH_PASSWORD: &str = "it-ssh-pw-1";
const SSH_IMAGE: &str = "lscr.io/linuxserver/openssh-server";

/// sshd and MongoDB on a private `cb-it-net-*` network. Everything is removed on drop
/// (containers first, then the network).
pub struct SshStack {
    network: String,
    mongo: String,
    sshd: String,
    pub ssh_port: u16,
}

impl SshStack {
    pub fn start() -> Self {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let mut stack = Self {
            network: format!("cb-it-net-{id}"),
            mongo: format!("cb-it-mongo-{id}"),
            sshd: format!("cb-it-sshd-{id}"),
            ssh_port: 0,
        };
        docker(&["network", "create", &stack.network]).expect("docker network create");
        docker(&[
            "run",
            "-d",
            "--name",
            &stack.mongo,
            "--network",
            &stack.network,
            MONGO_IMAGE,
        ])
        .expect("start mongo");
        let publish = "127.0.0.1::2222";
        let user = format!("USER_NAME={SSH_USER}");
        let pw = format!("USER_PASSWORD={SSH_PASSWORD}");
        docker(&[
            "run",
            "-d",
            "--name",
            &stack.sshd,
            "--network",
            &stack.network,
            "-p",
            publish,
            "-e",
            "PASSWORD_ACCESS=true",
            "-e",
            &user,
            "-e",
            &pw,
            SSH_IMAGE,
        ])
        .expect("start sshd");
        stack.ssh_port = published_port(&stack.sshd, "2222/tcp");
        stack.enable_forwarding();
        stack.wait_mongo();
        stack
    }

    /// The image ships with `AllowTcpForwarding no`; enable it and reload sshd.
    fn enable_forwarding(&self) {
        let script = "test -s /config/sshd.pid && \
            sed -i 's/^AllowTcpForwarding no/AllowTcpForwarding yes/' /config/sshd/sshd_config && \
            kill -HUP $(cat /config/sshd.pid)";
        poll(Duration::from_secs(60), "sshd config", || {
            docker(&["exec", &self.sshd, "sh", "-c", script]).is_ok()
        });
        // Give the reloaded daemon a moment to re-bind before the first dial.
        poll(Duration::from_secs(30), "sshd port", || {
            std::net::TcpStream::connect(("127.0.0.1", self.ssh_port)).is_ok()
        });
    }

    fn wait_mongo(&self) {
        let ping = "db.runCommand({ping:1}).ok";
        poll(Duration::from_secs(90), "mongo in ssh stack", || {
            docker(&["exec", &self.mongo, "mongosh", "--quiet", "--eval", ping])
                .is_ok_and(|out| out.ends_with('1'))
        });
    }

    /// The Mongo address as the sshd container sees it.
    pub fn target(&self) -> String {
        format!("{}:27017", self.mongo)
    }
}

impl Drop for SshStack {
    fn drop(&mut self) {
        for name in [&self.sshd, &self.mongo] {
            let _ = docker(&["rm", "-f", "-v", name]);
        }
        let _ = docker(&["network", "rm", &self.network]);
    }
}
