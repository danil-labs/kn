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
- `worktree finish` registra lo pendiente de la sesión antes de integrar y lo cuenta en `recorded_document_count`. «Guardar» queda para integrar; `commit` registra una versión dentro de la sesión.
- `KN_GIT` elige el ejecutable git y gana sobre PATH; una ruta inválida falla en vez de caer a PATH. Sin Git, el error `GIT_MISSING` nombra el remedio de cada sistema, e `init` falla antes de crear `.kn/` o KN_HOME.
- En Windows, los procesos que kn lanza usan `CREATE_NO_WINDOW`: lanzado sin consola, como lo hace Terminus, ya no abre una ventana por cada llamada a Git.
- Licencia doble: MIT o Apache-2.0, a elección de quien lo use (`LICENSE-MIT`, `LICENSE-APACHE`).
- La principal recuerda qué seguía en la nube en su última versión (`$KN_HOME/repos/<id>/cloud-pending.json`). `status` y `worktree add` informan `downloaded_since_last_observation`, y la observación registra lo descargado en su propia versión `cloud_download`, antes de los cambios manuales (`external_observation`).
- `kn cloud fetch [--timeout-secs N] [--max-bytes N]` descarga los documentos pendientes leyéndolos, del menor al mayor, con un límite de espera por documento. Devuelve `fetched`, `failed`, `skipped_budget`, `remaining` y `bytes_fetched`; no crea versiones.

### Límites de compatibilidad

La configuración local usa schema 2. `init --fresh` inicia otro historial y no importa versiones Go. Las sesiones son obligatorias para commit/restore; las carpetas vacías no se versionan. Nube, identidad personal, recuperación transaccional, releases automáticos y validación de Terminus siguen pendientes. El PR Go #1 queda como referencia y el issue #2 conserva sus pendientes.
