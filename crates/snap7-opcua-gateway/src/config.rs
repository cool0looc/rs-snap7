use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct GatewayConfig {
    pub opc_endpoint: String,
    #[serde(default = "default_poll_ms")]
    pub poll_interval_ms: u64,

    // --- Single-PLC (legacy) ---
    pub plc_addr: Option<String>,
    pub tags: Option<Vec<TagSpec>>,

    // --- Multi-PLC ---
    pub plcs: Option<Vec<PlcConfig>>,

    // --- OPC-UA security ---
    #[serde(default)]
    pub opc_security: OpcSecurityConfig,
}

fn default_poll_ms() -> u64 {
    1000
}

/// Configuration for a single PLC data source.
#[derive(Debug, Deserialize, Clone)]
pub struct PlcConfig {
    pub addr: String,
    #[serde(default = "default_poll_ms")]
    pub poll_interval_ms: u64,
    pub tags: Vec<TagSpec>,
}

/// OPC-UA security policy settings.
#[derive(Debug, Deserialize, Clone)]
pub struct OpcSecurityConfig {
    /// Security policy: "None" (default), "Basic128", "Basic256", "Basic256Sha256".
    #[serde(default = "default_security_policy")]
    pub policy: String,
    /// Message security mode: "None" (default), "Sign", "SignAndEncrypt".
    #[serde(default = "default_security_mode")]
    pub mode: String,
    /// Path to a PEM-encoded certificate file (required when mode != "None").
    pub certificate: Option<String>,
    /// Path to a PEM-encoded private key file.
    pub private_key: Option<String>,
}

impl Default for OpcSecurityConfig {
    fn default() -> Self {
        Self {
            policy: "None".into(),
            mode: "None".into(),
            certificate: None,
            private_key: None,
        }
    }
}

fn default_security_policy() -> String {
    "None".into()
}
fn default_security_mode() -> String {
    "None".into()
}

#[derive(Debug, Deserialize, Clone)]
pub struct TagSpec {
    pub tag: String,
    pub name: String,
    #[serde(default)]
    pub writable: bool,
}

impl GatewayConfig {
    /// Resolve all PLC configurations (merging legacy single-PLC with multi-PLC).
    pub fn plc_configs(&self) -> Vec<PlcConfig> {
        let mut out: Vec<PlcConfig> = self.plcs.clone().unwrap_or_default();
        // If legacy single-PLC fields exist and no multi-PLC, add as single entry
        if out.is_empty() {
            if let (Some(addr), Some(tags)) = (&self.plc_addr, &self.tags) {
                out.push(PlcConfig {
                    addr: addr.clone(),
                    poll_interval_ms: self.poll_interval_ms,
                    tags: tags.clone(),
                });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_plc_config() {
        let toml = r#"
            plc_addr = "127.0.0.1:102"
            opc_endpoint = "opc.tcp://0.0.0.0:4840"
            [[tags]]
            tag = "DB1,REAL4"
            name = "MotorSpeed"
        "#;
        let cfg: GatewayConfig = toml::from_str(toml).unwrap();
        let plcs = cfg.plc_configs();
        assert_eq!(plcs.len(), 1);
        assert_eq!(plcs[0].addr, "127.0.0.1:102");
        assert_eq!(plcs[0].tags.len(), 1);
    }

    #[test]
    fn parse_multi_plc_config() {
        let toml = r#"
            opc_endpoint = "opc.tcp://0.0.0.0:4840"
            [[plcs]]
            addr = "10.0.0.1:102"
            [[plcs.tags]]
            tag = "DB1,REAL0"
            name = "Temp1"
            [[plcs]]
            addr = "10.0.0.2:102"
            poll_interval_ms = 2000
            [[plcs.tags]]
            tag = "DB5,WORD0"
            name = "Pressure1"
        "#;
        let cfg: GatewayConfig = toml::from_str(toml).unwrap();
        let plcs = cfg.plc_configs();
        assert_eq!(plcs.len(), 2);
        assert_eq!(plcs[0].addr, "10.0.0.1:102");
        assert_eq!(plcs[0].tags.len(), 1);
        assert_eq!(plcs[1].addr, "10.0.0.2:102");
        assert_eq!(plcs[1].poll_interval_ms, 2000);
    }

    #[test]
    fn multi_plc_takes_precedence() {
        let toml = r#"
            plc_addr = "127.0.0.1:102"   # legacy — ignored when [[plcs]] present
            opc_endpoint = "opc.tcp://0.0.0.0:4840"
            [[plcs]]
            addr = "10.0.0.1:102"
            [[plcs.tags]]
            tag = "DB1,REAL0"
            name = "Tag1"
        "#;
        let cfg: GatewayConfig = toml::from_str(toml).unwrap();
        let plcs = cfg.plc_configs();
        assert_eq!(plcs.len(), 1);
        assert_eq!(plcs[0].addr, "10.0.0.1:102");
    }

    #[test]
    fn security_defaults() {
        let toml = r#"
            plc_addr = "127.0.0.1:102"
            opc_endpoint = "opc.tcp://0.0.0.0:4840"
            [[tags]]
            tag = "DB1,BYTE0"
            name = "Status"
        "#;
        let cfg: GatewayConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.opc_security.policy, "None");
        assert_eq!(cfg.opc_security.mode, "None");
    }

    #[test]
    fn security_with_cert() {
        let toml = r#"
            plc_addr = "127.0.0.1:102"
            opc_endpoint = "opc.tcp://0.0.0.0:4840"
            [[tags]]
            tag = "DB1,BYTE0"
            name = "Status"
            [opc_security]
            policy = "Basic256Sha256"
            mode = "SignAndEncrypt"
            certificate = "/etc/certs/server.pem"
            private_key = "/etc/certs/key.pem"
        "#;
        let cfg: GatewayConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.opc_security.policy, "Basic256Sha256");
        assert_eq!(cfg.opc_security.mode, "SignAndEncrypt");
        assert!(cfg.opc_security.certificate.is_some());
    }

    #[test]
    fn empty_plcs_returns_empty() {
        let cfg = GatewayConfig {
            opc_endpoint: "opc.tcp://0.0.0.0:4840".into(),
            poll_interval_ms: 1000,
            plc_addr: None,
            tags: None,
            plcs: None,
            opc_security: OpcSecurityConfig::default(),
        };
        assert!(cfg.plc_configs().is_empty());
    }
}
