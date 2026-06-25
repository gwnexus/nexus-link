# Security Policy

## Supported Versions

| Version | Supported |
| ------- | --------- |
| 0.7.x   | Yes       |
| < 0.7   | No        |

## Reporting a Vulnerability

If you discover a security vulnerability, please report it responsibly:

1. **Do not** open a public GitHub issue
2. Email **post+security@gatewarden.eu** with:
   - Description of the vulnerability
   - Steps to reproduce
   - Potential impact
   - Affected component (CLI, agent, service, core)
3. You will receive an acknowledgement within 48 hours
4. We will work with you to understand and address the issue before any
   public disclosure

## Scope

The following are in scope:

- Node token handling and storage (`nxs_node_*`)
- TLS configuration and certificate management
- Command authentication and signature verification
- Network communication (telemetry push, command reception)
- Local file system operations (config, credentials)
- Install script (`install.sh`)
- Docker image security (base images, runtime user)

## Out of Scope

- Denial of service against the Nexus backend (report to the platform team)
- Social engineering
- Physical access attacks

## Disclosure Policy

We follow coordinated disclosure. We ask that you give us reasonable time
(typically 90 days) to address the issue before making it public.

## Security Best Practices

When deploying nexus-link:

- Store node tokens with restricted file permissions (`chmod 600`)
- Use TLS certificates from a trusted CA for the service endpoint
- Run the service as a non-root user (Docker images use `nexus-link` user)
- Keep the binary updated to the latest release
- Monitor the service logs for unauthorized command attempts
