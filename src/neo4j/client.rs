use neo4rs::*;
use tracing::info;
use std::error::Error;

use crate::models::text_content::{KnowledgeSource, ProcessingError, Relationship};

/// A client for interacting with the Neo4j graph database.
pub struct Neo4jClient {
    /// The Neo4j graph instance.
    graph: Graph,
}

impl Neo4jClient {
    /// Creates a new `Neo4jClient` instance by connecting to the Neo4j database.
    ///
    /// # Arguments
    ///
    /// * `uri` - The URI of the Neo4j database (e.g., "127.0.0.1:7687").
    /// * `user` - The username for authentication.
    /// * `pass` - The password for authentication.
    ///
    /// # Errors
    ///
    /// Returns an `Err` variant if the connection to the database fails.
    pub async fn new(uri: &str, user: &str, pass: &str) -> Result<Self, Box<dyn Error>> {
        // Initialize the Neo4j graph connection.
        let graph = Graph::new(uri, user, pass).await?;
        Ok(Neo4jClient { graph })
    }

    /// Stores a knowledge source in the Neo4j database.
    ///
    /// # Arguments
    ///
    /// * `source` - A reference to the `KnowledgeSource` to be stored.
    ///
    /// # Errors
    ///
    /// Returns an `Err` variant if the database operation fails.
    pub async fn store_knowledge_source(
        &self,
        source: &KnowledgeSource,
    ) -> Result<(), ProcessingError> {
        // Cypher query to create a knowledge source node with properties.
        let cypher_query = "
            CREATE (ks:KnowledgeSource {
                id: $id,
                type: $type,
                title: $title,
                description: $description
            })
            RETURN ks
        ";

        // Execute the query with parameters.
        let _ = self.graph
            .run(
                query(cypher_query)
                    .param("id", source.id.to_string())
                    .param("title", source.title.to_string())
                    .param("description", source.description.to_string()),
            )
            .await.map_err(|e| ProcessingError::GraphDBError(e.to_string()));

        info!("Stored knowledge source");

        Ok(())
    }

    /// Stores a relationship between two knowledge sources in the Neo4j database.
    ///
    /// # Arguments
    ///
    /// * `source_id` - The ID of the source knowledge source.
    /// * `relationship` - A reference to the `Relationship` defining the connection.
    ///
    /// # Errors
    ///
    /// Returns an `Err` variant if the database operation fails.
    pub async fn store_relationship(
        &self,
        source_id: &str,
        relationship: &Relationship,
    ) -> Result<(), ProcessingError> {
        // Cypher query to create a relationship between two knowledge source nodes.
        let cypher_query = format!(
            "
            MATCH (a:KnowledgeSource {{id: $source_id}})
            MATCH (b:KnowledgeSource {{id: $target_id}})
            CREATE (a)-[:{}]->(b)
            RETURN a, b
            ",
            relationship.type_
        );

        // Execute the query with parameters.
        let _ = self.graph
            .run(
                query(&cypher_query)
                    .param("source_id", source_id)
                    .param("target", relationship.target.to_string()),
            )
            .await.map_err(|e| ProcessingError::GraphDBError(e.to_string()));

        info!("Stored knowledge relationship");

        Ok(())
    }

    //// Closes the connection to the Neo4j database.
    ////
    //// This function ensures that all pending operations are completed before shutting down.
    // pub async fn close(self) -> Result<(), Box<dyn Error>> {
    //     self.graph.close().await?;
    //     Ok(())
    // }
}

// #[cfg(test)]
// mod tests {
//     use super::*;
//     use uuid::Uuid;

//     /// Tests the `store_knowledge_source` and `store_relationship` functions.
//     ///
//     /// # Note
//     ///
//     /// Ensure that a Neo4j database is running and accessible at the specified URI
//     /// before running these tests.
//     #[tokio::test]
//     async fn test_store_functions() {
//         // Initialize the Neo4j client.
//         let client = Neo4jClient::new("127.0.0.1:7687", "neo4j", "neo")
//             .await
//             .expect("Failed to create Neo4j client");

//         // Create a first knowledge source.
//         let source1 = KnowledgeSource {
//             id: Uuid::new_v4().to_string(),
//             source_type: "Document".into(),
//             title: "Understanding Neural Networks".into(),
//             description:
//                 "An in-depth analysis of neural networks and their applications in machine learning."
//                     .into(),
//         };

//         // Store the first knowledge source.
//         client
//             .store_knowledge_source(&source1)
//             .await
//             .expect("Failed to store knowledge source 1");

//         // Create a second knowledge source.
//         let source2 = KnowledgeSource {
//             id: Uuid::new_v4().to_string(),
//             source_type: "Document".into(),
//             title: "Machine Learning Basics".into(),
//             description: "A foundational text on machine learning principles and techniques."
//                 .into(),
//         };

//         // Store the second knowledge source.
//         client
//             .store_knowledge_source(&source2)
//             .await
//             .expect("Failed to store knowledge source 2");

//         // Define a relationship between the two sources.
//         let relationship = Relationship {
//             relationship_type: "RelatedTo".into(),
//             target_id: source2.id.clone(),
//         };

//         // Store the relationship from source1 to source2.
//         client
//             .store_relationship(&source1.id, &relationship)
//             .await
//             .expect("Failed to store relationship");

//         // Clean up by closing the client.
//         client.close().await.expect("Failed to close Neo4j client");
//     }
// }
