# Changelog

## Sin publicar

- Remotos documentales mediante MCP ([PR #5](https://github.com/soydanil/kn/pull/5), [issue #6](https://github.com/soydanil/kn/issues/6)). Comandos nuevos: `remote add|list|show|verify|login|logout|remove`, `mode`, `fetch`, `pull` y `push [--dry-run] [--allow-deletes]`. `status --refresh` y `diff --remote|--base` ya funcionan. `connect` queda como alias oculto de `remote add`.
- Cliente MCP sobre Streamable HTTP, guiado por perfiles de herramientas revisados con esquemas fijados por SHA-256. Hay un perfil del servidor de referencia de las pruebas.
- Autorización de kn como cliente MCP:
  - metadata RFC 9728 y RFC 8414;
  - registro dinámico, PKCE S256 e indicador de recurso;
  - callback en loopback y renovación de tokens;
  - credenciales en el almacén del sistema o en `KN_MCP_ACCESS_TOKEN`.
- Modos de transferencia `local`, `desktop_sync`, `desktop_sync_observed` y `mcp`. Comparación B/L/R con Git y `merge-tree`, integración remota dentro de sesiones y publicación con precondiciones, diario y verificación por observación.
- Errores nuevos: REMOTE_NOT_CONFIGURED, MODE_FORBIDS_OPERATION, AUTH_REQUIRED, REMOTE_UNAVAILABLE, REMOTE_ERROR, REMOTE_INCOMPLETE y PROFILE_MISMATCH.

### Compatibilidad

- `status` agrega `data.remote`.
- `capabilities` refleja el modo actual y agrega `certified_providers`, que está vacío.
- `inspect` puede reportar `origin.connection: configured`.
- `pull`, `push`, `status --refresh` y `diff --base|--remote` ya no devuelven siempre `UNSUPPORTED_CAPABILITY`. Sin un modo que los permita, devuelven `MODE_FORBIDS_OPERATION` o `REMOTE_NOT_CONFIGURED`, también con exit 3.

Ningún proveedor está certificado. Drive, OneDrive y SharePoint requieren un servidor MCP seleccionado, su perfil y pruebas reales con dos cuentas.

## 0.3.0 — Sin release publicado

- Núcleo Rust `kn-core` y CLI `kn` para documentos con Git externo.
- Sesiones de agentes con worktrees, observaciones de cambios manuales, commits e integración fast-forward.
- Restauración con preversión y protección de archivos ignorados que obstruyen el objetivo.
- Consultas `rev-parse`, `status --porcelain` y `worktree list --porcelain`, con `-C` y soporte NUL.
- Nombres `commit`, `log`, `worktree add`; alias `snapshot`, `history`, `session start`.
- Envelope JSON 1.0 y errores operativos separados de conflictos y capacidades no disponibles.
- CI en Linux/macOS/Windows y documentación inicial de operación, arquitectura y contratos.

### Límites de compatibilidad

La configuración local usa schema 2. `init --fresh` inicia otro historial y no importa versiones Go. Las sesiones son obligatorias para commit/restore; las carpetas vacías no se versionan. Nube, identidad personal, recuperación transaccional, releases automáticos y validación de Terminus siguen pendientes. El PR Go #1 queda como referencia y el issue #2 conserva sus pendientes.
