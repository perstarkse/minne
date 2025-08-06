# Minne Integration Tests

This package contains comprehensive integration tests for Minne's core features. These tests validate the full stack functionality from HTTP endpoints through to database operations.

## Test Coverage

The integration tests cover the following key features:

### 1. Content Ingestion
- **Text Content Ingestion**: Tests the API endpoint for ingesting text content
- **URL Content Ingestion**: Tests the API endpoint for ingesting content from URLs
- **File Upload Ingestion**: Tests multipart form uploads with file attachments
- **Authentication**: Validates API key authentication and unauthorized access handling

### 2. Search Functionality
- **Search Interface**: Tests the HTML search interface
- **Vector Search**: Validates semantic search capabilities
- **Query Processing**: Tests various search query formats and parameters

### 3. Chat/LLM Features
- **Chat Initialization**: Tests chat interface loading
- **Message Submission**: Tests posting new chat messages
- **Conversation Management**: Tests conversation creation and management
- **LLM Integration**: Validates integration with OpenAI-compatible APIs

### 4. HTML Content Viewing
- **Content Display**: Tests viewing of processed HTML content
- **Content Rendering**: Validates proper HTML rendering and formatting
- **Content Retrieval**: Tests database retrieval of processed content

### 5. End-to-End Workflows
- **Ingestion to Search**: Complete workflow from content ingestion to searchability
- **Error Handling**: Tests various error conditions and edge cases
- **Authentication Flows**: Tests protected route access and session management

## Test Architecture

### Test Infrastructure
- **In-Memory Database**: Uses SurrealDB in-memory mode for isolated testing
- **Mock Services**: Mocks external dependencies like OpenAI API
- **Test Server**: Uses `axum-test` for HTTP endpoint testing
- **Isolated Tests**: Each test runs with a fresh database and configuration

### Test Utilities (`test_utils.rs`)
- `setup_test_server()`: Creates a complete test environment
- `setup_test_database()`: Initializes in-memory database with migrations
- `create_test_user()`: Creates test users with API keys
- `create_test_knowledge_entities()`: Seeds test data for search testing
- `create_test_html_entity()`: Creates test entities with HTML content

## Running the Tests

### Prerequisites
- Rust toolchain (1.70+)
- All workspace dependencies resolved

### Running All Tests
```bash
# From the workspace root
cargo test --package integration-tests

# Or from the integration-tests directory
cd integration-tests
cargo test
```

### Running Specific Tests
```bash
# Test only ingestion functionality
cargo test --package integration-tests test_ingestion

# Test only search functionality
cargo test --package integration-tests test_search

# Test only chat functionality
cargo test --package integration-tests test_chat

# Run with output for debugging
cargo test --package integration-tests -- --nocapture
```

### Running with Logging
```bash
# Enable logging during tests
RUST_LOG=debug cargo test --package integration-tests -- --nocapture
```

## Test Configuration

The tests use mock configurations that don't require external services:

- **Database**: In-memory SurrealDB (no external database needed)
- **OpenAI API**: Mock client with test API key
- **File Storage**: Temporary directories for test data
- **Sessions**: In-memory session storage

## Test Data

Tests create isolated test data including:
- Test users with API keys
- Sample knowledge entities
- Mock text content and HTML content
- Test categories and metadata

All test data is automatically cleaned up after each test.

## Extending the Tests

### Adding New Test Cases
1. Create new test functions in `integration_tests.rs`
2. Use the existing test utilities from `test_utils.rs`
3. Follow the existing patterns for setup and assertions

### Adding Test Utilities
1. Add helper functions to `test_utils.rs`
2. Document the function purpose and usage
3. Ensure proper cleanup of test resources

### Testing New Features
1. Identify the feature's HTTP endpoints
2. Create test scenarios covering normal and edge cases
3. Add database validation where appropriate
4. Include error handling tests

## Best Practices

1. **Test Isolation**: Each test should be completely independent
2. **Realistic Data**: Use realistic test data that matches production patterns
3. **Error Coverage**: Test both success and failure scenarios
4. **Performance**: Keep tests fast by using minimal test data
5. **Documentation**: Document complex test scenarios and their purpose

## Troubleshooting

### Common Issues
1. **Database Migration Errors**: Ensure all migrations are applied in test setup
2. **Authentication Failures**: Verify test users have proper API keys
3. **Template Errors**: Check that test environment includes necessary templates
4. **Session Issues**: Ensure session store is properly configured for tests

### Debugging Tests
- Use `RUST_LOG=debug` for detailed logging
- Add `println!` statements for debugging specific issues
- Use `cargo test -- --nocapture` to see test output
- Check that test database is properly isolated between tests

## Integration with CI/CD

These tests are designed to run in CI/CD environments:
- No external dependencies required
- Fast execution with in-memory database
- Comprehensive coverage of critical functionality
- Clear pass/fail indicators for automated testing