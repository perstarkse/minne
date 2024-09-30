use lapin::{
    options::*, publisher_confirm::Confirmation, BasicProperties,
};

use crate::models::ingress_object::IngressObject;

use super::{RabbitMQCommon, RabbitMQConfig, RabbitMQError};
use tracing::{info, error};

pub struct RabbitMQProducer {
    common: RabbitMQCommon,
    exchange_name: String,
    routing_key: String,
}

impl RabbitMQProducer {
    pub async fn new(config: &RabbitMQConfig) -> Result<Self, RabbitMQError> {
        let common = RabbitMQCommon::new(config).await?;
        common.declare_exchange(config, false).await?;

        Ok(Self { 
            common,
            exchange_name: config.exchange.clone(),
            routing_key: config.routing_key.clone(),
        })
    }

    /// Publishes an IngressObject to RabbitMQ after serializing it to JSON.
    pub async fn publish(&self, ingress_object: &IngressObject) -> Result<Confirmation, RabbitMQError> {
        // Serialize IngressObject to JSON
        let payload = serde_json::to_vec(ingress_object)
            .map_err(|e| {
                error!("Serialization Error: {}", e);
                RabbitMQError::PublishError(format!("Serialization Error: {}", e))
            })?;
        
        // Publish the serialized payload to RabbitMQ
        let confirmation = self.common.channel
            .basic_publish(
                &self.exchange_name,
                &self.routing_key,
                BasicPublishOptions::default(),
                &payload,
                BasicProperties::default(),
            )
            .await
            .map_err(|e| {
                error!("Publish Error: {}", e);
                RabbitMQError::PublishError(format!("Publish Error: {}", e))
            })?
            .await
            .map_err(|e| {
                error!("Publish Confirmation Error: {}", e);
                RabbitMQError::PublishError(format!("Publish Confirmation Error: {}", e))
            })?;
        
        info!("Published IngressObject to exchange '{}' with routing key '{}'", self.exchange_name, self.routing_key);
        
        Ok(confirmation)
    }
}
