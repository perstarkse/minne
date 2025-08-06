# Add S3-Compatible Object Storage Support

## Summary

This PR implements comprehensive S3-compatible object storage support for Minne, allowing users to store files in cloud object storage instead of (or in addition to) local filesystem storage. The implementation uses the `object_store` crate and supports a wide variety of S3-compatible providers.

## Key Features

### 🏗️ Storage Backend Abstraction
- **New `StorageBackend` trait**: Clean abstraction for different storage implementations
- **Factory pattern**: Easy configuration-based backend creation
- **Backward compatibility**: Existing filesystem functionality preserved

### 🪣 S3-Compatible Storage
- **AWS S3**: Full support with IAM roles and access keys
- **S3-Compatible Services**: MinIO, DigitalOcean Spaces, Backblaze B2, Cloudflare R2, etc.
- **Flexible authentication**: Environment variables, IAM roles, or explicit credentials
- **Custom endpoints**: Support for self-hosted and third-party S3-compatible services

### ⚙️ Configuration
- **Environment-based selection**: `STORAGE_BACKEND=filesystem|s3`
- **Comprehensive S3 options**: Bucket, region, endpoint, credentials, and prefix configuration
- **Sensible defaults**: Works out-of-the-box with minimal configuration

## Implementation Details

### New Files
- `common/src/storage/backends/mod.rs` - Storage backend abstractions and factory
- `common/src/storage/backends/filesystem.rs` - Filesystem backend implementation
- `common/src/storage/backends/object_storage.rs` - S3 backend implementation

### Modified Files
- `Cargo.toml` - Added `object_store` and `bytes` dependencies
- `common/Cargo.toml` - Added workspace dependencies
- `common/src/utils/config.rs` - Extended configuration with storage backend options
- `common/src/storage/types/file_info.rs` - Updated to use storage backend abstraction
- `common/src/storage/mod.rs` - Added backends module
- `README.md` - Comprehensive documentation updates

## Configuration Options

### Environment Variables

```bash
# Storage backend selection
STORAGE_BACKEND=s3  # or "filesystem" (default)

# S3 Configuration
S3_BUCKET=my-minne-bucket           # Required for S3
S3_REGION=us-east-1                 # Optional for custom endpoints
S3_ENDPOINT=https://s3.amazonaws.com # Optional, for S3-compatible services  
S3_ACCESS_KEY_ID=your-access-key    # Optional if using IAM roles
S3_SECRET_ACCESS_KEY=your-secret    # Optional if using IAM roles
S3_PREFIX=minne                     # Optional, defaults to "minne"
```

### config.yaml Example

```yaml
# Filesystem storage (default)
storage_backend: "filesystem"
data_dir: "./minne_app_data"

# Or S3 storage
storage_backend: "s3"
s3_bucket: "my-minne-bucket"
s3_region: "us-east-1"
s3_prefix: "minne"
```

## Testing

### Unit Tests
- ✅ Filesystem backend functionality
- ✅ S3 backend functionality (with integration test feature)
- ✅ Storage configuration factory
- ✅ File operations (store, retrieve, delete, metadata)

### Integration Tests
- S3 integration tests require real S3 credentials and are feature-gated
- Run with: `cargo test --features integration-tests`

## Backward Compatibility

### 🔄 Legacy Support
- **Existing APIs preserved**: `FileInfo::new_with_config()` for backward compatibility
- **Filesystem behavior unchanged**: Same directory structure and file handling
- **Database schema compatible**: No changes to existing FileInfo records
- **Migration support**: Clear documentation for storage backend migration

### 🚀 Migration Path
1. **Current users**: No changes required, continues using filesystem storage
2. **New S3 users**: Set `STORAGE_BACKEND=s3` and configure S3 options
3. **Migration**: Manual process documented in README (automated migration not implemented)

## Supported S3-Compatible Services

| Service | Configuration |
|---------|---------------|
| **AWS S3** | Set `S3_REGION`, optionally use IAM roles |
| **MinIO** | Set `S3_ENDPOINT` to MinIO server URL |
| **DigitalOcean Spaces** | Set `S3_ENDPOINT` to Spaces endpoint |
| **Backblaze B2** | Set `S3_ENDPOINT` to B2 S3 API endpoint |
| **Cloudflare R2** | Set `S3_ENDPOINT` to R2 endpoint |
| **Other S3-compatible** | Set appropriate `S3_ENDPOINT` |

## File Organization

### Filesystem
```
{data_dir}/{user_id}/{file_id}/{filename}
```

### S3
```
{bucket}/{prefix}/{user_id}/{file_id}/{filename}
```

## Security Considerations

- ✅ **Credential management**: Support for IAM roles, environment variables, and explicit credentials
- ✅ **Path safety**: Sanitized file names and proper path construction  
- ✅ **Access control**: S3 bucket permissions control access
- ✅ **Encryption**: Supports S3 server-side encryption options

## Performance

- 📈 **Async operations**: All storage operations are fully async
- 🔄 **Streaming**: Efficient file upload/download with streaming
- 🗜️ **Memory efficient**: Uses `Bytes` for zero-copy operations where possible
- 📊 **Metadata optimization**: Separate metadata operations to avoid unnecessary downloads

## Future Enhancements

- [ ] **Automatic migration**: Tools to migrate between storage backends
- [ ] **Multi-region support**: Replicate files across multiple S3 regions
- [ ] **CDN integration**: CloudFront or similar CDN support for file serving
- [ ] **Backup strategies**: Cross-backend replication for redundancy
- [ ] **Azure Blob/GCP support**: Additional cloud storage providers

## Breaking Changes

❌ **None** - This is a fully backward-compatible addition.

## Testing Instructions

### 1. Filesystem Backend (Default)
```bash
# No configuration changes needed
cargo test
```

### 2. S3 Backend
```bash
# Set up S3 credentials and bucket
export STORAGE_BACKEND=s3
export S3_BUCKET=your-test-bucket
export S3_REGION=us-east-1
export S3_ACCESS_KEY_ID=your-key
export S3_SECRET_ACCESS_KEY=your-secret

# Run with integration tests
cargo test --features integration-tests
```

### 3. S3-Compatible Service (e.g., MinIO)
```bash
export STORAGE_BACKEND=s3
export S3_BUCKET=test-bucket
export S3_ENDPOINT=http://localhost:9000
export S3_ACCESS_KEY_ID=minioadmin
export S3_SECRET_ACCESS_KEY=minioadmin
```

## Documentation Updates

- ✅ **README.md**: Comprehensive storage backend documentation
- ✅ **Configuration examples**: Both environment variables and config.yaml
- ✅ **Migration guide**: Steps for switching storage backends
- ✅ **Docker Compose examples**: Updated with S3 configuration
- ✅ **Service-specific guides**: Configuration for popular S3-compatible services

---

This implementation provides a robust, production-ready solution for object storage in Minne while maintaining full backward compatibility with existing installations.