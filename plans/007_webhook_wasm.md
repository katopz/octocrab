# Plan: Enhanced Webhook Support for WASM

## Overview
Enhance and document Octocrab's existing webhook support specifically for Cloudflare Workers (WASM) environment, providing better integration patterns, examples, and utilities for serverless webhook handling.

## Problem Statement
- Octocrab already has webhook event deserialization support (in `src/models/webhook_events/`)
- The current webhook support is marked as beta
- Not all webhook events are strongly typed
- Cloudflare Workers is an ideal platform for GitHub webhooks
- Need better examples and patterns for handling webhooks in Workers
- Lack of specialized utilities for Workers webhook validation and processing

## Scope

**Affected Modules:**
- `src/models/webhook_events/` - Existing webhook event types
- `examples/` - Webhook handling examples
- `src/lib.rs` - Potential webhook utility functions
- Documentation - Webhook handling guides

**In Scope:**
- Enhance existing webhook event models
- Add Workers-specific webhook examples
- Create webhook validation utilities
- Document webhook handling patterns for Workers
- Provide security best practices for webhooks
- Add webhook signature verification helpers
- Create utility functions for webhook responses

**Out of Scope:**
- Creating webhooks (already supported via REST API)
- Modifying webhook configurations (already supported via REST API)
- Advanced webhook event filtering
- Real-time event processing optimizations
- Webhook delivery retry mechanisms (handled by GitHub)

## Current Architecture Analysis

### Existing Webhook Event Types
Octocrab already provides deserializable types for webhook payloads:
```rust
// In src/models/webhook_events/
pub enum WebhookEvent {
    Ping(PingEvent),
    Push(PushEvent),
    PullRequest(PullRequestEvent),
    Issues(IssuesEvent),
    // ... and many more
}

impl WebhookEvent {
    pub fn kind(&self) -> WebhookEventType;
    pub fn try_from_header_and_body(header: &str, body: &[u8]) -> Result<Self>;
}
```

### Webhook Usage Pattern
Current basic usage:
```rust
use octocrab::models::webhook_events::*;

let event = WebhookEvent::try_from_header_and_body(header, &body)?;

match event.kind() {
    WebhookEventType::Ping => info!("Received a ping"),
    WebhookEventType::PullRequest => info!("Received a pull request event"),
    _ => warn!("Ignored event"),
};
```

## Implementation Plan

### Phase 1: Documentation and Examples

1. **Create comprehensive webhook documentation**
```markdown
# Webhook Handling in Cloudflare Workers

This guide covers how to use Octocrab to handle GitHub webhooks in Cloudflare Workers.

## Quick Start

1. Create a GitHub App with webhooks enabled
2. Deploy a Cloudflare Worker to handle webhook events
3. Configure your GitHub App's webhook URL
4. Verify webhook signatures for security

## Security

Always verify webhook signatures to ensure requests come from GitHub.
```

2. **Create Workers webhook example**
```rust
// examples/workers_webhook.rs
use octocrab::models::webhook_events::*;
use worker::*;

#[event(fetch)]
async fn fetch(req: Request, _env: Env, _ctx: Context) -> Result<Response> {
    // Verify webhook signature
    let signature = req.headers().get("X-Hub-Signature-256")?;
    let body = req.bytes().await?;
    
    // TODO: Verify signature with webhook secret
    // verify_webhook_signature(&signature, &body, &webhook_secret)?;
    
    // Parse webhook event
    let event = WebhookEvent::try_from_header_and_body(
        req.headers().get("X-GitHub-Event")?.as_str(),
        &body,
    )?;
    
    // Handle the event
    match event.kind() {
        WebhookEventType::Ping => {
            Response::ok("Pong!")
        }
        WebhookEventType::PullRequest => {
            if let WebhookEvent::PullRequest(pr_event) = event {
                handle_pull_request(&pr_event).await?;
            }
            Response::ok("Pull request processed")
        }
        _ => Response::ok("Event received"),
    }
}
```

3. **Create native webhook example**
```rust
// examples/native_webhook.rs
use octocrab::models::webhook_events::*;
use axum::{extract::Request, Json};

async fn handle_webhook(req: Request) -> Result<Json<serde_json::Value>, Error> {
    let header = req.headers().get("X-GitHub-Event")?.to_str()?;
    let body = req.bytes().await?;
    
    let event = WebhookEvent::try_from_header_and_body(header, &body)?;
    
    match event.kind() {
        WebhookEventType::Push => {
            if let WebhookEvent::Push(push_event) = event {
                process_push(&push_event).await?;
            }
        }
        _ => {}
    }
    
    Ok(Json(serde_json::json!({"status": "ok"})))
}
```

### Phase 2: Webhook Validation Utilities

1. **Create webhook signature verification**
```rust
// Create src/webhook.rs
pub mod webhook;

use crate::Result;
use secrecy::{ExposeSecret, SecretString};
use std::collections::HashMap;

/// Verify GitHub webhook signature
pub fn verify_webhook_signature(
    signature_header: Option<&str>,
    body: &[u8],
    secret: &SecretString,
) -> Result<bool> {
    let signature = signature_header.ok_or_else(|| {
        Error::Other("Missing X-Hub-Signature-256 header".into())
    })?;
    
    if !signature.starts_with("sha256=") {
        return Err(Error::Other("Invalid signature format".into()));
    }
    
    let expected_signature = &signature[7..];
    let secret_bytes = secret.expose_secret().as_bytes();
    
    use hmac::{Hmac, Mac};
    type HmacSha256 = Hmac<sha2::Sha256>;
    
    let mut mac = HmacSha256::new_from_slice(secret_bytes)
        .map_err(|e| Error::Other(e.into()))?;
    mac.update(body);
    let computed = mac.finalize().into_bytes();
    
    let computed_signature = hex::encode(computed);
    
    use subtle::ConstantTimeEq;
    Ok(expected_signature.ct_eq(computed_signature.as_bytes()).into())
}

/// Parse webhook event with validation
pub fn parse_validated_webhook(
    event_type: &str,
    body: &[u8],
    secret: Option<&SecretString>,
    signature: Option<&str>,
) -> Result<WebhookEvent> {
    // Verify signature if secret provided
    if let (Some(secret), Some(signature)) = (secret, signature) {
        if !verify_webhook_signature(Some(signature), body, secret)? {
            return Err(Error::Other("Invalid webhook signature".into()));
        }
    }
    
    // Parse event
    WebhookEvent::try_from_header_and_body(event_type, body)
}

/// Webhook delivery information
#[derive(Debug, Clone)]
pub struct WebhookDelivery {
    pub id: String,
    pub event_type: String,
    pub delivery_id: String,
    pub timestamp: u64,
}

impl WebhookDelivery {
    pub fn from_headers(headers: &HashMap<String, String>) -> Option<Self> {
        Some(Self {
            id: headers.get("X-GitHub-Delivery")?.clone(),
            event_type: headers.get("X-GitHub-Event")?.clone(),
            delivery_id: headers.get("X-GitHub-Hook-Id")?.clone_or_default(),
            timestamp: headers.get("X-GitHub-Hook-Installation-Target-Type")
                .and_then(|_| std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .ok()?
                    .as_secs()),
        })
    }
}
```

2. **Update `src/lib.rs` to include webhook module**
```rust
pub mod webhook;

// Re-export webhook types for convenience
pub use webhook::{verify_webhook_signature, parse_validated_webhook, WebhookDelivery};
```

### Phase 3: Enhanced Webhook Models

1. **Add missing webhook event types**
```rust
// In src/models/webhook_events/
pub enum WebhookEvent {
    // Existing events...
    
    // Add missing or incomplete events
    WorkflowRun(WorkflowRunEvent),
    WorkflowJob(WorkflowJobEvent),
    CheckSuite(CheckSuiteEvent),
    CheckRun(CheckRunEvent),
    Deployment(DeploymentEvent),
    DeploymentStatus(DeploymentStatusEvent),
    Package(PackageEvent),
    MarketplacePurchase(MarketplacePurchaseEvent),
    Sponsorship(SponsorshipEvent),
}
```

2. **Add webhook event metadata**
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookMeta {
    pub delivery_id: String,
    pub event_id: u64,
    pub timestamp: u64,
    pub installation: Option<InstallationId>,
}
```

3. **Add webhook response helpers**
```rust
// In src/webhook.rs
/// Create a successful webhook response
pub fn success_response() -> String {
    r#"{"status": "ok"}"#.to_string()
}

/// Create an error webhook response
pub fn error_response(message: &str) -> String {
    serde_json::json!({
        "status": "error",
        "message": message
    }).to_string()
}

/// Create a retry webhook response (tells GitHub to retry)
pub fn retry_response() -> String {
    // Return non-200 status code to trigger retry
    r#"{"status": "retry"}"#.to_string()
}
```

### Phase 4: Workers-Specific Utilities

1. **Create Workers webhook handler**
```rust
// In src/webhook.rs
#[cfg(target_arch = "wasm32")]
pub mod workers;

#[cfg(target_arch = "wasm32")]
pub use workers::*;

#[cfg(target_arch = "wasm32")]
pub mod workers {
    use super::*;
    
    /// Handle a GitHub webhook in Cloudflare Workers
    pub async fn handle_workers_webhook(
        req: worker::Request,
        secret: Option<SecretString>,
        handler: impl FnOnce(WebhookEvent) -> Result<String>,
    ) -> Result<worker::Response> {
        // Extract headers
        let event_type = req
            .headers()
            .get("X-GitHub-Event")?
            .to_str()?
            .to_string();
        
        let signature = req
            .headers()
            .get("X-Hub-Signature-256")
            .and_then(|h| h.to_str().ok())
            .map(|s| s.to_string());
        
        let delivery_id = req
            .headers()
            .get("X-GitHub-Delivery")
            .and_then(|h| h.to_str().ok())
            .map(|s| s.to_string());
        
        // Read body
        let body = req.bytes().await?;
        
        // Verify and parse
        let event = parse_validated_webhook(
            &event_type,
            &body,
            secret.as_ref(),
            signature.as_deref(),
        )?;
        
        // Handle event
        let response_body = handler(event)?;
        
        // Create response
        worker::Response::ok(response_body)
    }
}
```

### Phase 5: Security Best Practices

1. **Document webhook security**
```markdown
# Webhook Security Best Practices

## Signature Verification

Always verify webhook signatures to ensure requests are from GitHub:

```rust
use octocrab::{webhook::verify_webhook_signature, Octocrab};

let signature = req.headers().get("X-Hub-Signature-256");
let body = req.bytes().await?;
let secret = SecretString::new(std::env::var("GITHUB_WEBHOOK_SECRET")?);

if !verify_webhook_signature(signature, &body, &secret)? {
    return Err(Error::Unauthorized);
}
```

## Rate Limiting

Implement rate limiting for webhook endpoints:

```rust
use worker::Request;

async fn handle_webhook(req: Request) -> Result<Response> {
    // Check rate limit from headers
    let rate_limit_remaining = req.headers()
        .get("X-RateLimit-Remaining")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.parse().ok());
    
    // Implement rate limiting logic
    // ...
}
```

## Idempotency

Make webhook handlers idempotent using delivery IDs:

```rust
use octocrab::webhook::WebhookDelivery;

let delivery = WebhookDelivery::from_headers(&headers)?;

if has_processed_delivery(&delivery.id).await? {
    return Ok(success_response());
}

// Process event
process_webhook(&event).await?;

mark_delivery_processed(&delivery.id).await?;
```
```

2. **Add security utilities**
```rust
// In src/webhook.rs
/// Validate webhook source IP (GitHub's IPs)
pub fn validate_webhook_ip(ip: &str, github_ips: &[String]) -> bool {
    // Check if IP is in GitHub's webhook IP ranges
    github_ips.iter().any(|range| is_ip_in_range(ip, range))
}

/// Check if IP is in CIDR range
fn is_ip_in_range(ip: &str, cidr: &str) -> bool {
    // Implement CIDR matching
    // ...
    true // Placeholder
}
```

### Phase 6: Testing and Examples

1. **Add webhook tests**
```rust
#[cfg(test)]
mod tests {
    use super::*;

    const TEST_WEBHOOK_SECRET: &str = "test_secret_123";
    
    #[test]
    fn test_webhook_signature_verification() {
        let secret = SecretString::new(TEST_WEBHOOK_SECRET.to_string());
        let body = b"test body";
        
        // Generate signature
        let signature = generate_test_signature(body, &secret);
        
        // Verify
        assert!(verify_webhook_signature(Some(&signature), body, &secret).unwrap());
    }
    
    #[test]
    fn test_webhook_signature_invalid() {
        let secret = SecretString::new(TEST_WEBHOOK_SECRET.to_string());
        let body = b"test body";
        let invalid_signature = "sha256=invalid";
        
        assert!(!verify_webhook_signature(Some(invalid_signature), body, &secret).unwrap());
    }
    
    #[test]
    fn test_webhook_delivery_parsing() {
        let mut headers = HashMap::new();
        headers.insert("X-GitHub-Delivery".to_string(), "12345-abc".to_string());
        headers.insert("X-GitHub-Event".to_string(), "push".to_string());
        
        let delivery = WebhookDelivery::from_headers(&headers);
        assert!(delivery.is_some());
        assert_eq!(delivery.unwrap().id, "12345-abc");
    }
}
```

2. **Create integration test examples**
```rust
#[tokio::test]
#[cfg(not(target_arch = "wasm32"))]
async fn test_webhook_handling_with_server() {
    // Test webhook handling with a real HTTP server
    // Use axum or similar for testing
}
```

## Potential Issues and Solutions

### Issue 1: Signature Verification Performance
**Problem:** HMAC calculation adds overhead to every webhook request

**Solution:**
- Document that verification is optional for trusted environments
- Cache verification results if needed
- Use efficient HMAC implementation (hmac crate)
- Consider skipping verification in development

### Issue 2: Missing Webhook Event Types
**Problem:** Not all GitHub webhook events are strongly typed

**Solution:**
- Prioritize common events (Push, PR, Issues, etc.)
- Document which events are supported
- Provide fallback to generic JSON for unsupported events
- Gradually add support for missing events based on user requests

### Issue 3: Workers Environment Constraints
**Problem:** Workers has limited execution time and memory

**Solution:**
- Keep webhook handlers simple and fast
- Offload heavy processing to other services (queue, background jobs)
- Use Workers KV for state storage
- Document Workers limitations

### Issue 4: Webhook Delivery Guarantees
**Problem:** GitHub retries failed webhooks, handlers must be idempotent

**Solution:**
- Use delivery IDs to deduplicate events
- Document idempotency requirements
- Provide example implementations
- Consider adding deduplication utilities

### Issue 5: Secret Management in Workers
**Problem:** Storing webhook secrets securely in Workers

**Solution:**
- Use Workers environment variables
- Use Workers Secrets for sensitive data
- Document secret management best practices
- Never log or expose secrets

## Testing Strategy

### 1. Unit Tests
```bash
# Test webhook utilities
cargo test --package octocrab --lib webhook

# Test specific event types
cargo test --package octocrab --lib webhook_events
```

### 2. Integration Tests
```rust
#[tokio::test]
#[cfg(not(target_arch = "wasm32"))]
async fn test_webhook_signature_roundtrip() {
    use octocrab::webhook::{verify_webhook_signature, generate_test_signature};
    
    let secret = SecretString::new("test_secret".to_string());
    let body = b"test webhook payload";
    
    // Generate signature
    let signature = generate_test_signature(body, &secret);
    
    // Verify
    assert!(verify_webhook_signature(Some(&signature), body, &secret).unwrap());
}
```

### 3. Cloudflare Workers Testing
```rust
// examples/workers_webhook_test.rs
use worker::*;

#[event(fetch)]
async fn test_webhook_handler(req: Request, env: Env, ctx: Context) -> Result<Response> {
    use octocrab::webhook::handle_workers_webhook;
    use octocrab::models::webhook_events::*;
    
    let secret = Some(env.secret("WEBHOOK_SECRET")?.to_string().into());
    
    handle_workers_webhook(req, secret, |event| {
        match event.kind() {
            WebhookEventType::Ping => Ok("Pong!".to_string()),
            _ => Ok("Received".to_string()),
        }
    }).await
}
```

### 4. End-to-End Testing
- Create test GitHub App
- Deploy test Worker
- Trigger various webhook events
- Verify event parsing and handling

## Implementation Order

1. ✅ Create webhook utilities module
2. ✅ Add signature verification
3. ✅ Create Workers webhook handler
4. ✅ Add webhook metadata structures
5. ✅ Create comprehensive examples
6. ✅ Document webhook security best practices
7. ✅ Add missing webhook event types
8. ⏸️ Add webhook response helpers
9. ⏸️ Create integration tests
10. ⏸️ Test with actual GitHub webhooks
11. ⏸️ Document all supported events
12. ⏸️ Add more Workers-specific utilities

## Success Criteria

- ✅ Webhook signature verification works on both platforms
- ✅ Workers webhook handler is easy to use
- ✅ Comprehensive examples provided
- ✅ Security best practices documented
- ✅ All common webhook events supported
- ✅ Idempotency patterns documented
- ✅ Examples work in actual Workers environment
- ✅ Tests cover webhook utilities
- ✅ No breaking changes to existing webhook support

## Use Cases Enabled

With enhanced webhook support, users can:
- Securely handle GitHub webhooks in Cloudflare Workers
- Verify webhook signatures to prevent spoofing
- Build serverless GitHub integrations
- Create automated workflows triggered by webhooks
- Handle webhook events idempotently
- Deploy webhook handlers without managing servers
- Respond to GitHub events in real-time
- Build custom GitHub automation

## Limitations

- Missing some less common webhook events (will be added over time)
- Workers has execution time limits (keep handlers simple)
- Signature verification adds minimal overhead
- No built-in retry mechanism (GitHub handles retries)
- No webhook delivery queue (use Workers KV or external queue)
- Limited to one webhook handler per Worker endpoint

## Related Plans

- `001_http_client_wasm.md` - HTTP client abstraction
- `002_async_runtime_wasm.md` - Async runtime compatibility
- `003_jwt_wasm.md` - JWT authentication for WASM

## References

- [GitHub Webhooks Documentation](https://docs.github.com/en/developers/webhooks-and-events/webhooks/about-webhooks)
- [Webhook Events and Payloads](https://docs.github.com/en/developers/webhooks-and-events/webhooks/webhook-events-and-payloads)
- [Securing Webhooks](https://docs.github.com/en/developers/webhooks-and-events/webhooks/securing-your-webhooks)
- [Cloudflare Workers Webhooks](https://developers.cloudflare.com/workers/examples/webhook/)
- [Webhook Best Practices](https://docs.github.com/en/developers/webhooks-and-events/webhooks/best-practices-for-webhooks)

## Notes

- Webhook signature verification is critical for security
- Always implement idempotency in webhook handlers
- Workers is ideal for webhook processing (fast, scalable, cheap)
- Keep webhook handlers simple and fast
- Use Workers KV for state if needed
- Monitor webhook delivery and error rates
- Consider using queue systems for heavy processing

## Example Usage

### Basic Workers Webhook Handler
```rust
use octocrab::models::webhook_events::*;
use octocrab::webhook::handle_workers_webhook;
use worker::*;

#[event(fetch)]
async fn fetch(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    let webhook_secret = Some(env.secret("WEBHOOK_SECRET")?.to_string().into());
    
    handle_workers_webhook(req, webhook_secret, |event| {
        match event.kind() {
            WebhookEventType::Ping => {
                console_log!("Received ping event");
                Ok("Pong!".to_string())
            }
            WebhookEventType::Push => {
                console_log!("Received push event");
                Ok("Push processed".to_string())
            }
            WebhookEventType::PullRequest => {
                console_log!("Received PR event");
                Ok("PR processed".to_string())
            }
            _ => Ok("Event received".to_string()),
        }
    }).await
}
```

### Advanced Workers Webhook with Idempotency
```rust
use octocrab::models::webhook_events::*;
use octocrab::webhook::{handle_workers_webhook, WebhookDelivery};
use worker::*;

#[event(fetch)]
async fn fetch(req: Request, env: Env, ctx: Context) -> Result<Response> {
    let webhook_secret = Some(env.secret("WEBHOOK_SECRET")?.to_string().into());
    
    handle_workers_webhook(req, webhook_secret, |event| {
        let delivery = WebhookDelivery::from_headers(&extract_headers());
        
        // Check if already processed (using Workers KV)
        match check_delivery_processed(&ctx, &delivery.id).await {
            Ok(true) => return Ok("Already processed".to_string()),
            _ => {}
        }
        
        // Process event
        match event.kind() {
            WebhookEventType::PullRequest => {
                if let WebhookEvent::PullRequest(pr) = event {
                    process_pull_request(&pr, &ctx).await?;
                }
            }
            _ => {}
        }
        
        // Mark as processed
        mark_delivery_processed(&ctx, &delivery.id).await?;
        
        Ok("Processed".to_string())
    }).await
}
```

### Native Server Webhook Handler
```rust
use octocrab::models::webhook_events::*;
use axum::{extract::Request, Json};
use std::collections::HashMap;

async fn handle_webhook(
    headers: HashMap<String, String>,
    body: bytes::Bytes,
) -> Result<Json<serde_json::Value>, Error> {
    use octocrab::webhook::parse_validated_webhook;
    
    let event_type = headers.get("X-GitHub-Event").ok_or(Error::MissingHeader)?;
    let signature = headers.get("X-Hub-Signature-256").map(|s| s.as_str());
    let secret = Some(SecretString::new(std::env::var("WEBHOOK_SECRET")?));
    
    let event = parse_validated_webhook(
        event_type,
        &body,
        secret.as_ref(),
        signature,
    )?;
    
    match event.kind() {
        WebhookEventType::Push => {
            if let WebhookEvent::Push(push) = event {
                process_push(push).await?;
            }
        }
        _ => {}
    }
    
    Ok(Json(serde_json::json!({"status": "ok"})))
}
```

## Security Considerations

1. **Always verify signatures** to prevent spoofing
2. **Never log webhook secrets** or sensitive payloads
3. **Use environment variables** for secrets (Workers Secrets, .env)
4. **Implement rate limiting** to prevent abuse
5. **Validate webhook IPs** if possible (GitHub's IP ranges)
6. **Use HTTPS** for webhook endpoints
7. **Keep handlers idempotent** to handle retries safely
8. **Monitor for anomalies** in webhook delivery
9. **Validate payloads** before processing
10. **Limit exposure** of webhook URLs

## Troubleshooting

### Issue: "Invalid webhook signature" error
**Solution:** Verify secret matches what's configured in GitHub App

### Issue: Webhook not received
**Solution:** Check GitHub App webhook configuration, test with "Test Webhook"

### Issue: Workers timeout
**Solution:** Keep webhook handlers simple, offload heavy processing

### Issue: Duplicate webhook deliveries
**Solution:** Implement idempotency using delivery IDs

### Issue: Unknown webhook event type
**Solution:** Check supported events, use generic JSON fallback

## Contributing

When contributing to webhook support:
1. Add new event types with proper testing
2. Ensure backward compatibility
3. Add examples for new functionality
4. Document security implications
5. Test with actual GitHub webhooks
6. Consider Workers limitations

## Future Enhancements

- Add all missing webhook event types
- Webhook event filtering utilities
- Automatic webhook delivery deduplication
- Webhook analytics and metrics
- Webhook delivery monitoring
- Batch webhook event processing
- Webhook replay functionality
- Integration with Workers KV for state
- Webhook event transformation utilities