# Changelog

## 0.3.0 — Sin release publicado

- Núcleo Rust `kn-core` y CLI `kn` para documentos con Git externo.
- Sesiones de agentes con worktrees, observaciones de cambios manuales, commits e integración fast-forward.
- Restauración con preversión y protección de archivos ignorados que obstruyen el objetivo.
- Consultas `rev-parse`, `status --porcelain` y `worktree list --porcelain`, con `-C` y soporte NUL.
- Nombres `commit`, `log`, `worktree add`; alias `snapshot`, `history`, `session start`.
- Envelope JSON 1.0 y errores operativos separados de conflictos y capacidades no disponibles.
- CI en Linux/macOS/Windows y documentación inicial de operación, arquitectura y contratos.
- Documentos que siguen en la nube: `init`, las observaciones y las sesiones continúan sin descargarlos, los reportan en `cloud_only` y los versionan cuando el proveedor los descarga.
- Un `init` que falla ya no deja un historial huérfano en KN_HOME por cada reintento.

### Límites de compatibilidad

La configuración local usa schema 2. `init --fresh` inicia otro historial y no importa versiones Go. Las sesiones son obligatorias para commit/restore; las carpetas vacías no se versionan. Nube, identidad personal, recuperación transaccional, releases automáticos y validación de Terminus siguen pendientes. El PR Go #1 queda como referencia y el issue #2 conserva sus pendientes.
