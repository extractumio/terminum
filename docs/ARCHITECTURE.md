# terminum's architecture

One picture of every part: terminumd on a Mac, the tmux servers it drives (its own, the user's,
a Linux server's over ssh) and the browsers that show them. Behavior and limits live in the
specs ([docs/specs/](specs/)); this page only maps the parts.

```mermaid
flowchart LR
  subgraph Clients["Browsers"]
    SM["Safari on the Mac"]
    CM["Chrome on the Mac"]
    IP["Safari on an iPhone"]
  end

  subgraph Mac["Mac (macOS)"]
    TS["tailscaled: tailscale serve HTTPS :443"]
    LD["launchd: agent io.terminum.terminumd, holds 127.0.0.1:48080, starts at login and after a crash"]
    TCC["macOS privacy (TCC): folder answers for io.terminum.terminumd"]
    CLI["terminum command: install, upgrade, status, folders, password"]
    subgraph TMD["terminumd at ~/.terminum/daemon/terminumd"]
      WEB["HTTP and WebSocket: the Page and the Files panel embedded, password sign-in"]
      CC["tmux control clients: tmux -C, one per session"]
      FS["files: listings, watcher, saves under Roots, Trash"]
      SOCK["CLI socket: state/terminumd.sock"]
      ST["state: settings.json, workspaces, layout, projects, unread"]
      SSHC["ssh links to hosts"]
    end
    subgraph LT["tmux -L terminum, started by terminumd"]
      SH1["shells: bash or zsh"]
      AG1["claude, codex and their subagents"]
    end
    UT["the user's own tmux, sessions attached by name (optional)"]
  end

  subgraph Linux["Linux server"]
    SSHD["sshd"]
    RT["tmux -C: the host session terminum"]
    RA["terminumd --agent: that host's files"]
    subgraph RS["its tmux sessions"]
      SH2["shells"]
      AG2["claude, codex"]
    end
  end

  GH["GitHub Releases: signed packages, the daily version check"]

  SM -->|"http://127.0.0.1:48080"| LD
  CM -->|"http://127.0.0.1:48080"| LD
  IP -->|"HTTPS over the tailnet"| TS
  TS -->|"to 127.0.0.1:48080"| LD
  LD -->|"hands over the socket"| WEB
  CLI --> SOCK
  CLI -->|"terminum upgrade"| GH
  WEB -->|"screens and keys"| CC
  CC -->|"notifications in, commands out"| LT
  CC --> UT
  SH1 --> AG1
  AG1 -.->|"Documents, Desktop, cloud folders"| TCC
  FS -.-> TCC
  WEB --> FS
  WEB --> ST
  SSHC -->|"ssh"| SSHD
  SSHD --> RT
  SSHD --> RA
  RT --> RS
  SH2 --> AG2
  WEB -.->|"a host's screens and files"| SSHC
  TMD -.->|"copies its agent build there"| RA
```

- **Browsers** reach terminumd on `127.0.0.1:48080`, which launchd holds; other devices come
  through `tailscale serve` over HTTPS. Every browser signs in with the password set by
  `terminum password` (SECURITY.md).
- **terminumd** is the only client of every tmux server: its control clients (`tmux -C`) read
  screens and layouts as notifications and send keys and commands. It serves the Page and the
  Files panel, reads and writes files under its Roots and keeps the state in `~/.terminum/state`.
- **Folder access on a Mac**: macOS judges every program in terminum's terminals by the program
  that started the tmux server, `daemon/terminumd`, so one Allow covers them all (AC-08).
- **A Linux server** is reached over ssh: its tmux in control mode for the terminals, and a
  `terminumd --agent` for its files, which terminumd copies there and keeps at its own version.
- **terminumd on a Linux server** is the same picture with systemd's user service
  `io.terminum.terminumd` in launchd's place (terminumd binds the port itself, `KillMode=process`
  keeps the terminals through every restart) and no macOS privacy prompts; its page is reached
  through an ssh tunnel or `tailscale serve` on the server.
