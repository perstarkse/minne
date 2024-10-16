# SurrealDB only

Right now we have the FileInfo stored in "files"

- Change the uuid to Uuid type, and have the database layer still use String. Means parsing and unparsing but thats fine.

```
pub struct FileInfo {
    pub uuid: String,
    pub sha256: String,
    pub path: String,
    pub mime_type: String,
}
```

We create TextContent objects, which we should store?

- We store the "snippets" along with the vectors, but it would make sense to store the whole textcontent, at least for not enormous files?

```
pub struct TextContent {
    pub id: Uuid,
    pub text: String,
    pub file_info: Option<FileInfo>,
    pub instructions: String,
    pub category: String,
}
```

We create KnowledgeSource, which we will store

- Add a uuid to we can link the textcontent and files to the knowledge sources?

```
pub struct KnowledgeSource {
    pub id: String,
    pub title: String,
    pub description: String,
    pub relationships: Vec<Relationship>,
}
```

We will create embeddings and vector representations of TextContent, possibly split up and store in vector DB

```
pub struct VectorEmbeddingOfTextContent {
    pub id: Uuid,
    pub vectors: Vec<u8>(or something),
    pub text_content: String,
    pub category: String,
}
```

______________________________________________________________________

## Goals

- Smooth operations when updating, removing and adding data
- Smooth queries where one can search, get a vector snippet, which links to a graph node and its edges, and also the fulltext document.
