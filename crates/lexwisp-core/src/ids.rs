use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! stable_id {
    ($name:ident) => {
        #[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4().to_string())
            }

            pub fn parse(value: impl Into<String>) -> Result<Self, String> {
                let value = value.into();
                Uuid::parse_str(&value)
                    .map(|_| Self(value))
                    .map_err(|error| format!("invalid {}: {error}", stringify!($name)))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

stable_id!(ConversationId);
stable_id!(MessageId);
stable_id!(AttemptId);
stable_id!(InvocationId);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_ids_are_stable_uuid_strings() {
        let id = ConversationId::new();
        assert_eq!(ConversationId::parse(id.to_string()), Ok(id));
    }
}
