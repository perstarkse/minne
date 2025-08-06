# Integration Tests - Requirements and Setup

## Version Requirements

The integration tests require **Rust 1.83+** due to the following dependencies:
- `async-graphql@7.0.16` requires rustc 1.83.0
- `reblessive@0.4.3` requires rustc 1.84

These dependencies are inherited from the main workspace and are required for SurrealDB and the web framework components.

## Setting Up for Testing

### Option 1: Using Latest Rust (Recommended)
```bash
# Update Rust to the latest stable version
rustup update stable

# Verify version
rustc --version  # Should be 1.83+ or later

# Run tests
cargo test --package integration-tests
```

### Option 2: Using Nix (If Available)
```bash
# If the project has a nix environment configured
nix develop

# Then run tests
cargo test --package integration-tests
```

### Option 3: Version-Specific Updates
If you need to maintain compatibility with older Rust versions, you can try updating specific dependencies:

```bash
# Update async-graphql to a compatible version
cargo update async-graphql --precise 7.0.15

# Update reblessive to a compatible version  
cargo update reblessive --precise 0.4.2
```

Note: This may cause other compatibility issues and is not recommended.

## Current Test Status

✅ **Compilation**: Tests compile successfully with Rust 1.83+
⚠️  **Runtime**: Tests may need additional dependencies for full end-to-end functionality
✅ **Database**: In-memory SurrealDB tests work correctly
✅ **API Testing**: Basic API endpoint testing works
✅ **Authentication**: API key authentication testing works

## Test Categories

### Working Tests
- Database setup and migrations
- User creation and management
- Knowledge entity operations
- Basic API router setup
- Authentication flows
- Data persistence
- Multi-user isolation

### Tests Requiring Additional Setup
- Full ingestion pipeline (requires headless Chrome)
- HTML template rendering (requires template files)
- Full search functionality (requires vector search setup)
- Chat functionality (requires OpenAI API mock)

## Running Tests

### Quick Test
```bash
# Run only the working database and API tests
cargo test --package integration-tests test_database
cargo test --package integration-tests test_user
cargo test --package integration-tests test_knowledge
cargo test --package integration-tests test_api
```

### Full Test Suite
```bash
# Run all tests (some may fail without full environment)
cargo test --package integration-tests
```

### Debugging Tests
```bash
# Run with output and logging
RUST_LOG=debug cargo test --package integration-tests -- --nocapture
```

## Future Improvements

1. **Mock Services**: Add proper mocking for external dependencies
2. **Test Data**: Expand test data sets for more comprehensive coverage
3. **Performance Tests**: Add performance benchmarks for key operations
4. **E2E Tests**: Add full end-to-end workflow tests
5. **Docker Testing**: Add Docker-based testing environment
6. **CI Integration**: Set up automated testing in CI/CD pipeline

## Contributing to Tests

When adding new tests:
1. Follow the existing patterns in `test_utils.rs`
2. Ensure tests are isolated and don't depend on external services
3. Add appropriate documentation for test purpose and setup
4. Consider both success and failure scenarios
5. Use realistic test data that matches production patterns