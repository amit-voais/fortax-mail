These self-signed certificates and the accompanying private key are public test
fixtures, never production credentials. They permit deterministic local TLS
handshake tests. `server.pem` names localhost and 127.0.0.1; `wrong-host.pem` names
wrong.example; and `proton-bridge.pem` mirrors Proton Bridge's self-signed,
CA-marked endpoint certificate. Their long expiration is intentional to avoid
time-sensitive CI.
