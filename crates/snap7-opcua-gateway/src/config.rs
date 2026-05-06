use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct GatewayConfig {
    pub plc_addr: String,
    pub opc_endpoint: String,
    #[serde(default = "default_poll_ms")]
    pub poll_interval_ms: u64,
    pub tags: Vec<TagSpec>,
}

fn default_poll_ms() -> u64 {
    1000
}

#[derive(Debug, Deserialize, Clone)]
pub struct TagSpec {
    pub tag: String,
    pub name: String,
    #[serde(default)]
    pub writable: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_config() {
        let toml = r#"
            plc_addr = "127.0.0.1:102"
            opc_endpoint = "opc.tcp://0.0.0.0:4840"
            poll_interval_ms = 500
            [[tags]]
            tag = "DB1,REAL4"
            name = "MotorSpeed"
        "#;
        let cfg: GatewayConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.tags.len(), 1);
        assert_eq!(cfg.tags[0].name, "MotorSpeed");
        assert_eq!(cfg.tags[0].tag, "DB1,REAL4");
    }

    #[test]
    fn missing_plc_addr_returns_err() {
        let toml = r#"
            opc_endpoint = "opc.tcp://0.0.0.0:4840"
            poll_interval_ms = 500
            [[tags]]
            tag = "DB1,BYTE0"
            name = "Status"
        "#;
        assert!(toml::from_str::<GatewayConfig>(toml).is_err());
    }

    #[test]
    fn default_poll_interval() {
        let toml = r#"
            plc_addr = "127.0.0.1:102"
            opc_endpoint = "opc.tcp://0.0.0.0:4840"
            [[tags]]
            tag = "DB1,BYTE0"
            name = "Status"
        "#;
        let cfg: GatewayConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.poll_interval_ms, 1000); // default
    }

    #[test]
    fn writable_tag_defaults_to_false() {
        let toml = r#"
            plc_addr = "127.0.0.1:102"
            opc_endpoint = "opc.tcp://0.0.0.0:4840"
            [[tags]]
            tag = "DB1,REAL0"
            name = "Temperature"
        "#;
        let cfg: GatewayConfig = toml::from_str(toml).unwrap();
        assert!(!cfg.tags[0].writable);
    }

    #[test]
    fn multiple_tags_parse_correctly() {
        let toml = r#"
            plc_addr = "192.168.1.100:102"
            opc_endpoint = "opc.tcp://0.0.0.0:4840"
            poll_interval_ms = 200
            [[tags]]
            tag = "DB1,REAL0"
            name = "MotorSpeed"
            [[tags]]
            tag = "DB2,WORD10"
            name = "PressureSetpoint"
            writable = true
        "#;
        let cfg: GatewayConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.tags.len(), 2);
        assert_eq!(cfg.tags[0].name, "MotorSpeed");
        assert!(!cfg.tags[0].writable);
        assert_eq!(cfg.tags[1].name, "PressureSetpoint");
        assert!(cfg.tags[1].writable);
    }
}
