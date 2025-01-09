use crate::ingress::types::ingress_object::IngressObject;
use serde::Serialize;

#[derive(Serialize)]
pub struct QueueTask {
    pub delivery_tag: u64,
    pub content: IngressObject,
}

#[derive(Serialize)]
pub struct QueueTaskResponse {
    pub tasks: Vec<QueueTask>,
}
