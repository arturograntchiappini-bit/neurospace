use std::fmt;
use std::net::SocketAddr;
use std::str::FromStr;

use crate::error::{Result, SyncError};
use crate::state::{DeviceId, Secret, from_hex32, to_hex};

/// What one device hands another to join its sync group: `ns1/<id hex>/<secret hex>[/<addr>,<addr>]`.
///
/// A ticket is a password: anyone holding it can read and write the vault.
#[derive(Clone, PartialEq, Eq)]
pub struct Ticket {
    /// Device id of the issuing device.
    pub device_id: DeviceId,
    /// The group secret.
    pub secret: Secret,
    /// Addresses the issuing device can be reached at; may be empty.
    pub addrs: Vec<SocketAddr>,
}

impl fmt::Debug for Ticket {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.debug_struct("Ticket")
            .field("device_id", &to_hex(&self.device_id))
            .field("secret", &"<redacted>")
            .field("addrs", &self.addrs)
            .finish()
    }
}

impl fmt::Display for Ticket {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            out,
            "ns1/{}/{}",
            to_hex(&self.device_id),
            to_hex(&self.secret)
        )?;
        if self.addrs.is_empty() {
            return Ok(());
        }
        let addrs: Vec<String> = self.addrs.iter().map(ToString::to_string).collect();
        write!(out, "/{}", addrs.join(","))
    }
}

impl FromStr for Ticket {
    type Err = SyncError;

    fn from_str(text: &str) -> Result<Self> {
        let bad = |why: &str| SyncError::InvalidTicket(why.to_string());
        let parts: Vec<&str> = text.split('/').collect();
        let (["ns1", id, secret] | ["ns1", id, secret, _]) = parts.as_slice() else {
            return Err(bad("expected ns1/<device id>/<secret>[/<addresses>]"));
        };
        let device_id = from_hex32(id).ok_or_else(|| bad("device id is not 64 hex characters"))?;
        let secret = from_hex32(secret).ok_or_else(|| bad("secret is not 64 hex characters"))?;
        let addrs = parts
            .get(3)
            .map(|list| {
                list.split(',')
                    .map(str::parse)
                    .collect::<std::result::Result<Vec<_>, _>>()
            })
            .transpose()
            .map_err(|_| bad("an address is not ip:port"))?
            .unwrap_or_default();
        Ok(Ticket {
            device_id,
            secret,
            addrs,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticket(addrs: &[&str]) -> Ticket {
        Ticket {
            device_id: [0x11; 32],
            secret: [0x22; 32],
            addrs: addrs.iter().map(|a| a.parse().unwrap()).collect(),
        }
    }

    #[test]
    fn should_print_and_parse_ticket() {
        let base = format!("ns1/{}/{}", "11".repeat(32), "22".repeat(32));
        let bare = ticket(&[]);
        assert_eq!(bare.to_string(), base);
        let full = ticket(&["192.168.1.5:4000", "[::1]:9"]);
        let text = full.to_string();
        assert_eq!(text, format!("{base}/192.168.1.5:4000,[::1]:9"));
        assert_eq!(base.parse::<Ticket>().unwrap(), bare);
        assert_eq!(text.parse::<Ticket>().unwrap(), full);
        let letters = Ticket {
            device_id: [0xab; 32],
            secret: [0xcd; 32],
            addrs: vec![],
        };
        let upper = format!("ns1/{}/{}", "AB".repeat(32), "CD".repeat(32));
        assert_eq!(upper.parse::<Ticket>().unwrap(), letters);
    }

    #[test]
    fn should_redact_secret_in_debug_output() {
        let issued = Ticket {
            device_id: [0x11; 32],
            secret: [0x5e; 32],
            addrs: vec!["1.2.3.4:5".parse().unwrap()],
        };
        let shown = format!("{issued:?}");
        assert!(!shown.contains(&"5e".repeat(32)), "{shown}");
        assert!(shown.contains(&"11".repeat(32)), "{shown}");
        assert!(shown.contains("1.2.3.4:5"), "{shown}");
        assert!(shown.contains(r#"secret: "<redacted>""#), "{shown}");
    }

    #[test]
    fn should_reject_malformed_tickets() {
        let id = "11".repeat(32);
        let secret = "22".repeat(32);
        let bad = [
            String::new(),
            format!("ns2/{id}/{secret}"),
            format!("ns1/{}/{secret}", &id[..63]),
            format!("ns1/{id}/{}", "zz".repeat(32)),
            format!("ns1/{id}/{secret}/"),
            format!("ns1/{id}/{secret}/not-an-address"),
            format!("ns1/{id}/{secret}/1.2.3.4:5/extra"),
        ];
        for text in bad {
            assert!(
                matches!(text.parse::<Ticket>(), Err(SyncError::InvalidTicket(_))),
                "accepted {text:?}"
            );
        }
    }
}
