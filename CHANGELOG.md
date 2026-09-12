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
- La identidad de una principal vive en `$KN_HOME/roots.json` (ruta canónica → id) y no en la carpeta. `init` ya no crea `.kn/` ni su lock, que pasa a `$KN_HOME/locks/`. Una carpeta de Drive u OneDrive ya no lleva a otras máquinas un marcador sin historial, y nadie la rompe para su dueña con `init --fresh`.
- Carpetas anidadas con historiales independientes: lo que integra la de abajo llega a la de arriba como `external_observation`. Inicializar dentro de una sesión sigue rechazado.
- `kn migrate` registra una principal con `.kn/config.json` y quita su `.kn`; devuelve `registered`, `removed` y `workspace_id`. Las carpetas con marcador siguen funcionando y se registran en su siguiente escritura. Un marcador cuyo historial no está en esta máquina ya no bloquea `init`.
- `cloud fetch --max-bytes 0` no lee ningún documento, tampoco los de tamaño aparente cero.
- `kn cloud fetch --all` sigue por tandas hasta que no queda nada que intentar, con `--max-bytes` como tope total. `--progress` escribe en stderr una línea JSON por documento; stdout conserva el único envelope. Así una aplicación puede lanzar la descarga en segundo plano y mostrar el avance sin reimplementarla.
- Las descargas que fallan se recuerdan en `cloud-pending.json` (schema 2, `failed: [{path, reason, at}]`) y las siguientes las omiten salvo con `--retry-failed`. `status` devuelve `cloud_failed` y `cloud_only_bytes`.
- Dos `cloud fetch` del mismo historial no corren a la vez: la segunda termina con `CLOUD_FETCH_BUSY`.
- README documenta que `kn-core` se puede usar directamente como librería desde otra aplicación Rust, sin pasar por `kn` como subproceso ni por su salida `--json` (véase «Usar kn-core como librería»).

### Límites de compatibilidad

El registro usa schema 1 y los marcadores de sesión, schema 2. `cloud-pending.json` pasa a schema 2: kn lee el 1, pero un kn anterior no lee el 2. Mover o renombrar una principal registrada la deja sin identidad: `init` en la nueva ruta empieza otro historial. `init --fresh` inicia otro historial y no importa versiones Go. Las sesiones son obligatorias para commit/restore; las carpetas vacías no se versionan. Nube, identidad personal, recuperación transaccional, releases automáticos y validación de Terminus siguen pendientes. El PR Go #1 queda como referencia y el issue #2 conserva sus pendientes.
