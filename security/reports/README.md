# Generated security evidence

Run `./scripts/security-audit.sh all` to generate `security-audit.json`,
`security-audit.md`, command logs and the CycloneDX SBOM here. Generated files
are ignored by Git; CI uploads them as artifacts. Reports can contain paths,
dependency metadata or redacted secret findings, so review before sharing.
