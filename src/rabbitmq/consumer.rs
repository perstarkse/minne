use lapin::{
    message::Delivery, options::*, types::FieldTable, Channel, Consumer, Queue 
};
use futures_lite::stream::StreamExt;

use crate::models::{ingress_content::IngressContentError, ingress_object::IngressObject};

use super::{RabbitMQCommon, RabbitMQConfig, RabbitMQError};
use tracing::{info, error};

/// Struct to consume messages from RabbitMQ.
pub struct RabbitMQConsumer {
    common: RabbitMQCommon,
    pub queue: Queue,
    consumer: Consumer,
}

impl RabbitMQConsumer {
    //// Creates a new 'RabbitMQConsumer' instance which sets up a rabbitmq client,
    //// declares a exchange if needed, declares and binds a queue and initializes the consumer
    ////
    //// # Arguments
    ////
    //// * 'config' - RabbitMQConfig
    ////
    //// # Returns
    ////
    //// * 'Result<Self, RabbitMQError>' - The created client or an error.
    pub async fn new(config: &RabbitMQConfig) -> Result<Self, RabbitMQError> {
        let common = RabbitMQCommon::new(config).await?;
        
        // Passively declare the exchange (it should already exist)
        common.declare_exchange(config, true).await?;
        
        // Declare queue and bind it to the channel
        let queue = Self::declare_queue(&common.channel, config).await?;
        Self::bind_queue(&common.channel, &config.exchange, &queue, config).await?;
        
        // Initialize the consumer
        let consumer = Self::initialize_consumer(&common.channel, &config).await?;

        Ok(Self { common, queue, consumer })
    }

    async fn initialize_consumer(channel: &Channel, config: &RabbitMQConfig) -> Result<Consumer, RabbitMQError> {
        channel
            .basic_consume(
                &config.queue,
                "consumer",
                BasicConsumeOptions::default(),
                FieldTable::default(),
            )
            .await.map_err(|e| RabbitMQError::InitializeConsumerError(e.to_string()))
    }

    async fn declare_queue(channel: &Channel, config: &RabbitMQConfig) -> Result<Queue, RabbitMQError> {
        channel
            .queue_declare(
                &config.queue,
                QueueDeclareOptions {
                    durable: true,
                    ..QueueDeclareOptions::default()
                },
                FieldTable::default(),
            )
            .await
            .map_err(|e| RabbitMQError::QueueError(e.to_string()))
    }

    async fn bind_queue(channel: &Channel, exchange: &str, queue: &Queue, config: &RabbitMQConfig) -> Result<(), RabbitMQError> {
        channel
            .queue_bind(
                queue.name().as_str(),
                exchange,
                &config.routing_key,
                QueueBindOptions::default(),
                FieldTable::default(),
            )
            .await
            .map_err(|e| RabbitMQError::QueueError(e.to_string()))
    }

    /// Consumes a message and returns the deserialized IngressContent along with the Delivery
    pub async fn consume(&self) -> Result<(IngressObject, Delivery), RabbitMQError> {
        // Receive the next message
        let delivery = self.consumer.clone().next().await
            .ok_or_else(|| RabbitMQError::ConsumeError("No message received".to_string()))?
            .map_err(|e| RabbitMQError::ConsumeError(e.to_string()))?;

        // Deserialize the message payload into IngressContent
        let ingress: IngressObject = serde_json::from_slice(&delivery.data)
            .map_err(|e| RabbitMQError::ConsumeError(format!("Deserialization Error: {}", e)))?;

        Ok((ingress, delivery))
    }

    /// Acknowledges the message after processing
    pub async fn ack_delivery(&self, delivery: Delivery) -> Result<(), RabbitMQError> {
        self.common.channel
            .basic_ack(delivery.delivery_tag, BasicAckOptions::default())
            .await
            .map_err(|e| RabbitMQError::ConsumeError(e.to_string()))?;

        Ok(())
    }
    pub async fn process_messages(&self) -> Result<(), RabbitMQError> {
        loop {
            match self.consume().await {
                Ok((ingress, delivery)) => {
                    info!("Received IngressObject: {:?}", ingress);

                    self.ack_delivery(delivery).await?;
                    // Process the IngressContent
                    // match self.handle_ingress_content(&ingress).await {
                    //     Ok(_) => {
                    //         info!("Successfully handled IngressContent");
                    //         // Acknowledge the message
                    //         if let Err(e) = self.ack_delivery(delivery).await {
                    //             error!("Failed to acknowledge message: {:?}", e);
                    //         }
                    //     },
                    //     Err(e) => {
                    //         error!("Failed to handle IngressContent: {:?}", e);
                    //         // For now, we'll acknowledge to remove it from the queue. Change to nack?
                    //         if let Err(ack_err) = self.ack_delivery(delivery).await {
                    //             error!("Failed to acknowledge message after handling error: {:?}", ack_err);
                    //         }
                    //     }
                    // }
                }
                Err(RabbitMQError::ConsumeError(e)) => {
                    error!("Error consuming message: {}", e);
                    // Optionally add a delay before trying again
                    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                }
                Err(e) => {
                    error!("Unexpected error: {}", e);
                    break;
                }
            }
        }

        Ok(())
    }

    // /// Processes messages in a loop
    // pub async fn process_messages(&self) -> Result<(), RabbitMQError> {
    //     loop {
    //         match self.consume().await {
    //             Ok((ingress, delivery)) => {
    //                 // info!("Received ingress object: {:?}", ingress);
                    
    //                 // Process the ingress object
    //                 self.handle_ingress_content(&ingress).await;

    //                 info!("Processing done, acknowledging message");
    //                 self.ack_delivery(delivery).await?;
    //             }
    //             Err(RabbitMQError::ConsumeError(e)) => {
    //                 error!("Error consuming message: {}", e);
    //                 // Optionally add a delay before trying again
    //                 tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    //             }
    //             Err(e) => {
    //                 error!("Unexpected error: {}", e);
    //                 break;
    //             }
    //         }
    //     }

    //     Ok(())
    // }
     pub async fn handle_ingress_content(&self, ingress: &IngressObject) -> Result<(), IngressContentError> {
        info!("Processing IngressContent: {:?}", ingress);

        

        // Convert IngressContent to individual TextContent instances
        // let text_contents = ingress.to_text_contents().await?;
        // info!("Generated {} TextContent instances", text_contents.len());

        // // Limit concurrent processing (e.g., 10 at a time)
        // let semaphore = Arc::new(Semaphore::new(10));
        // let mut processing_futures = FuturesUnordered::new();

        // for text_content in text_contents {
        //     let semaphore_clone = semaphore.clone();
        //     processing_futures.push(tokio::spawn(async move {
        //         let _permit = semaphore_clone.acquire().await;
        //         match text_content.process().await {
        //             Ok(_) => {
        //                 info!("Successfully processed TextContent");
        //                 Ok(())
        //             }
        //             Err(e) => {
        //                 error!("Error processing TextContent: {:?}", e);
        //                 Err(e)
        //             }
        //         }
        //     }));
        // }

        // // Await all processing tasks
        // while let Some(result) = processing_futures.next().await {
        //     match result {
        //         Ok(Ok(_)) => {
        //             // Successfully processed
        //         }
        //         Ok(Err(e)) => {
        //             // Processing failed, already logged
        //         }
        //         Err(e) => {
        //             // Task join error
        //             error!("Task join error: {:?}", e);
        //         }
        //     }
        // }

        // Ok(())
        unimplemented!()
}
}

    // /// Handles the IngressContent based on its type
    // async fn handle_ingress_content(&self, ingress: &IngressContent) {
    //     info!("Processing content: {:?}", ingress);
    //     // There are three different situations: 
    //     // 1. There is no ingress.content but there are one or more files
    //     //    - We should process the files and act based upon the mime type
    //     //    - All different kinds of content return text
    //     // 2. There is ingress.content but there are no files
    //     //    - We process ingress.content differently if its a URL or Text enum
    //     //    - Return text
    //     // 3. There is ingress.content and files
    //     //    - We do both
    //     // 
    //     // At the end of processing we have one or several text objects with some associated
    //     // metadata, such as the FileInfo metadata, or the Url associated to the text
    //     // 
    //     // There will always be ingress.instructions and ingress.category
    //     // 
    //     // When we have all the text objects and metadata, we can begin the next processing
    //     // Here we will:
    //     // 1. Send the text content and metadata to a LLM for analyzing
    //     //    - We want several things, JSON_LD metadata, possibly actions
    //     // 2. Store the JSON_LD in a graph database
    //     // 3. Split up the text intelligently and store it in a vector database
    //     //
    //     // We return the function if all succeeds.
    // }

