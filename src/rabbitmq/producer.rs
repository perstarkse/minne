use lapin::{
    options::*, publisher_confirm::Confirmation, BasicProperties,    
};

use super::{RabbitMQCommon, RabbitMQConfig, RabbitMQError};

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

    pub async fn publish(&self, payload: &[u8]) -> Result<Confirmation, RabbitMQError> {
        self.common.channel
            .basic_publish(
                &self.exchange_name,
                &self.routing_key,
                BasicPublishOptions::default(),
                payload,
                BasicProperties::default(),
            )
            .await
            .map_err(|e| RabbitMQError::PublishError(e.to_string()))?
            .await
            .map_err(|e| RabbitMQError::PublishError(e.to_string()))
    }
}
