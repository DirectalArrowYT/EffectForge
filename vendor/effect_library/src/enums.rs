use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Version {
    pub major: u8,
    pub minor: u8,
    pub patch: u8,
    pub build: u8,
}

impl Version {
    pub fn from_raw(raw: u32, _is_64_bit: bool) -> Option<Self> {
        // Version encoding: major | minor | patch | build
        let major = (raw >> 24) as u8;
        let minor = ((raw >> 16) & 0xFF) as u8;
        let patch = ((raw >> 8) & 0xFF) as u8;
        let build = (raw & 0xFF) as u8;

        Some(Version {
            major,
            minor,
            patch,
            build,
        })
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum TextureFormat {
    R8G8B8A8Unorm = 0,
}

#[derive(Debug, Clone, Copy)]
pub enum ColorType {
    Constant,
    Random,
    Animated8Key,
    Unknown(u8),
}

impl ColorType {
    pub fn as_u8(&self) -> u8 {
        match self {
            ColorType::Constant => 0,
            ColorType::Random => 1,
            ColorType::Animated8Key => 2,
            ColorType::Unknown(value) => *value,
        }
    }

    pub fn name(&self) -> Option<&'static str> {
        match self {
            ColorType::Constant => Some("Constant"),
            ColorType::Random => Some("Random"),
            ColorType::Animated8Key => Some("Animated8Key"),
            ColorType::Unknown(_) => None,
        }
    }

    pub fn from_u8(val: u8) -> Self {
        match val {
            1 => ColorType::Random,
            2 => ColorType::Animated8Key,
            0 => ColorType::Constant,
            _ => ColorType::Unknown(val),
        }
    }
}

impl Serialize for ColorType {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if let Some(name) = self.name() {
            serializer.serialize_str(name)
        } else {
            serializer.serialize_u8(self.as_u8())
        }
    }
}

impl<'de> serde::Deserialize<'de> for ColorType {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::String(s) => match s.as_str() {
                "Constant" => Ok(ColorType::Constant),
                "Random" => Ok(ColorType::Random),
                "Animated8Key" => Ok(ColorType::Animated8Key),
                other => Err(Error::custom(format!("unknown ColorType: {other}"))),
            },
            serde_json::Value::Number(n) => Ok(ColorType::from_u8(n.as_u64().unwrap_or(0) as u8)),
            _ => Err(Error::custom("invalid ColorType")),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum WrapMode {
    Mirror,
    Repeat,
    ClampEdge,
    MirrorOnce,
    Unknown(u8),
}

impl WrapMode {
    pub fn as_u8(&self) -> u8 {
        match self {
            WrapMode::Mirror => 0,
            WrapMode::Repeat => 1,
            WrapMode::ClampEdge => 2,
            WrapMode::MirrorOnce => 3,
            WrapMode::Unknown(value) => *value,
        }
    }

    pub fn name(&self) -> Option<&'static str> {
        match self {
            WrapMode::Mirror => Some("Mirror"),
            WrapMode::Repeat => Some("Repeat"),
            WrapMode::ClampEdge => Some("ClampEdge"),
            WrapMode::MirrorOnce => Some("MirrorOnce"),
            WrapMode::Unknown(_) => None,
        }
    }

    pub fn from_u8(val: u8) -> Self {
        match val {
            1 => WrapMode::Repeat,
            2 => WrapMode::ClampEdge,
            3 => WrapMode::MirrorOnce,
            0 => WrapMode::Mirror,
            _ => WrapMode::Unknown(val),
        }
    }
}

impl Serialize for WrapMode {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if let Some(name) = self.name() {
            serializer.serialize_str(name)
        } else {
            serializer.serialize_u8(self.as_u8())
        }
    }
}

impl<'de> serde::Deserialize<'de> for WrapMode {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::String(s) => match s.as_str() {
                "Mirror" => Ok(WrapMode::Mirror),
                "Repeat" => Ok(WrapMode::Repeat),
                "ClampEdge" => Ok(WrapMode::ClampEdge),
                "MirrorOnce" => Ok(WrapMode::MirrorOnce),
                other => Err(Error::custom(format!("unknown WrapMode: {other}"))),
            },
            serde_json::Value::Number(n) => Ok(WrapMode::from_u8(n.as_u64().unwrap_or(0) as u8)),
            _ => Err(Error::custom("invalid WrapMode")),
        }
    }
}
