use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
struct APIKey {
    #[serde(alias = "name")]
    id: String,
    #[serde(alias = "api_key")]
    key: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct Usage {
    #[serde(alias = "name")]
    key_id: String,

    // TODO: see https://docs.python.org/3.8/library/time.html#time.time
    time: f32,
}
