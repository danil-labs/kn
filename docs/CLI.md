# Contrato CLI

Fuente de implementación: [main.rs](../crates/kn/src/main.rs), [plumbing.rs](../crates/kn-core/src/plumbing.rs), [error.rs](../crates/kn-core/src/error.rs). Versión del binario: Cargo.toml; versión de envelope JSON: `1.0`. No es una implementación completa de todos los comandos Git.

## Nombres y operaciones

| Comando | Resultado |
| --- | --- |
| `init [--fresh]` | Inicializa; fresh asigna otra identidad, no convierte el historial anterior |
| `status [--refresh]` | Cambios, conflictos, capacidades y estado remoto almacenado; refresh observa antes el remoto activo |
| `diff [--patch] [--base\|--remote]` | Cambios desde HEAD; con base/remote, versión actual frente a la base reconciliada o la última observación remota |
| `commit [-m mensaje]` | Guarda todos los documentos permitidos de la sesión; alias snapshot |
| `log [--limit N] [--offset N]` | Versiones documentales, limit 1–1000, offset desde cero; alias history |
| `restore v_<12 hex>` | Guarda antes los cambios y restaura dentro de la sesión |
| `worktree add <nombre>` | Crea sesión bajo KN_HOME; alias session start |
| `worktree list` | Lista sesiones conservadas; con porcelain incluye principal y registros Git |
| `worktree update` | Integra la principal en la sesión actual |
| `worktree finish` | Integra la sesión a la principal si puede avanzar fast-forward |
| `remote add <alias> <url> --profile <archivo> --root <id> [--account <ref>]` | Guarda el remoto y una copia del perfil bajo KN_HOME; no contacta al servidor. `connect` es alias oculto |
| `remote list`, `remote show <alias>` | Remotos configurados; configuración y último estado conocido, sin red |
| `remote verify <alias>` | Conecta, compara herramientas y esquemas con el perfil y comprueba la raíz; nunca escribe |
| `remote login <alias>`, `remote logout <alias>` | Autoriza kn ante el servidor y guarda la credencial en el almacén del sistema; la borra |
| `remote remove <alias>` | Quita un remoto inactivo sin publicaciones pendientes |
| `mode`, `mode set <modo> [--remote <alias>] [--primary-outside-sync]` | Consulta o cambia quién transfiere: `local`, `desktop_sync`, `desktop_sync_observed`, `mcp`; nunca transfiere documentos |
| `fetch` | Observa completo el remoto activo; no cambia los documentos de la principal |
| `pull` | En una sesión: observa e integra el remoto con Git; los conflictos quedan en la sesión |
| `push [--dry-run] [--allow-deletes]` | Solo en modo `mcp`: publica el commit actual de main con precondiciones y lo verifica |
| `version`, `--version` | Versión del build |

`-C <carpeta>` fija el directorio de ejecución; se admite una sola ocurrencia. No cambia el cwd del consumidor. `--help` funciona por comando. El nombre de sesión acepta 1–64 letras ASCII, números, guiones y guiones bajos; no recibe una ruta arbitraria.

## Consultas Git para consumidores

`rev-parse` requiere exactamente una consulta:

| Consulta | stdout con terminador LF |
| --- | --- |
| `--is-inside-work-tree` | `true` dentro de kn; fuera de kn, error con stdout vacío |
| `--show-toplevel` | Ruta absoluta de la principal o sesión que contiene el cwd |
| `--git-dir` | Ruta absoluta del directorio Git de ese worktree |
| `--git-common-dir` | Ruta absoluta del historial compartido entre esas sesiones locales |
| `HEAD` | Identificador completo del commit, distinto al ID visible `v_…` |

`--is-inside-workspace` queda como alias de `--is-inside-work-tree`. Una carpeta Git ajena a kn no cuenta como un espacio kn; el descubrimiento no cruza repositorios Git anidados. Un fallo de permisos/corrupción no implica «carpeta de código». Las consultas conservan las rutas de documentos locales; no prueban si el origen está en nube.

`status --porcelain [-z]` transmite la salida porcelain v1 de Git. El formato incluye XY de estado, nombres y registros de rename según Git. `worktree list --porcelain [-z]` transmite registros worktree/HEAD/branch de Git, corrigiendo la primera ruta con la principal registrada por kn, porque el directorio Git es externo. No cambia las demás etiquetas de Git.

Con `-z`, los separadores son bytes NUL; el consumidor debe procesar bytes, no hacer split por líneas ni trim de rutas. Sin `-z`, Git usa su quoting y la principal adaptada sigue quoting C. No se admite `--porcelain=v2` ni otras opciones no presentes en ayuda. `-z` requiere `--porcelain` explícito.

Estas consultas usan un lock compartido existente y no crean estado ni versiones. Admiten nombres con acentos y espacios. La adaptación de rutas de rev-parse exige UTF-8. No mezclan mensajes humanos o envelope JSON con stdout de máquina; `--json` junto a rev-parse/porcelain es entrada inválida.

## Envelope JSON de aplicación

Los comandos de aplicación aceptan `--json`. Un único objeto se escribe a stdout, sin diagnósticos duplicados en stderr:

```json
{
  "schema_version": "1.0",
  "operation_id": "f9de11e4-cd49-4ca5-b3d8-f69144c281fb",
  "status": "ok",
  "data": {},
  "conflicts": [],
  "errors": []
}
```

[JSON Schema del envelope](schema/envelope-v1.json). `data` depende del comando y todavía no tiene un esquema publicado por operación. El mensaje humano es explicativo y no debe parsearse. En errores, data es null; cada error contiene `code`, `message`, `retryable` y `suggested_next_action`. Se marcan retryable WORKSPACE_BUSY, REMOTE_UNAVAILABLE y REMOTE_INCOMPLETE.

| Exit | status | Interpretación |
| --- | --- | --- |
| 0 | ok | Operación o ayuda exitosa |
| 1 | error | Error operativo: IO, Git, estado, identidad o bloqueo |
| 2 | conflict | Integración o restauración necesita intervención |
| 3 | unsupported | Entrada inválida, sesión requerida o capacidad no implementada; consultar code |

Los códigos numéricos son de kn, no una reproducción exacta de los de Git. En modo humano o porcelain los errores van a stderr. En JSON, incluso los errores de argumentos llevan envelope. Los programas deben revisar exit y error.code, no solo el texto o status.

| code | Situación |
| --- | --- |
| INVALID_INPUT | Argumentos, formato de versión o schema no admitido |
| UNSUPPORTED_CAPABILITY | Cloud u otra operación no implementada |
| NOT_A_WORKSPACE | No se descubrió un registro kn |
| WORKSPACE_BUSY | Se agotó la espera de lock |
| WORKSPACE_COPIED | La misma identidad apunta a otra ubicación todavía existente |
| WORKSPACE_IDENTITY_CHANGED | Una sesión antigua intenta acceder a una principal de otro espacio |
| SESSION_REQUIRED | Escritura de documentos fuera de una sesión |
| CONFLICT | Integración divergente, conflictos o archivos no versionados que obstruyen |
| UNSAFE_PATH | Registro, symlink o ruta que viola las restricciones |
| GIT_FAILED | Falló el proceso Git |
| IO_ERROR | Falló filesystem/proceso |
| INVALID_STATE | Estado JSON no se puede deserializar |
| REMOTE_NOT_CONFIGURED | El modo no nombra un remoto activo (exit 3) |
| MODE_FORBIDS_OPERATION | El modo de transferencia no permite la operación; se rechaza antes de contactar al servidor (exit 3) |
| AUTH_REQUIRED | El servidor exige autorización o rechazó la credencial |
| REMOTE_UNAVAILABLE | Red, HTTP 5xx, sesión expirada o respuesta perdida: resultado desconocido, se verifica observando |
| REMOTE_ERROR | El servidor respondió un error o datos que no cumplen el perfil |
| REMOTE_INCOMPLETE | La observación no fue completa o la verificación no mostró la publicación; no se infieren borrados |
| PROFILE_MISMATCH | El esquema de una herramienta cambió respecto al perfil (exit 3) |

## Remotos MCP

Diseño y límites: [remotos mediante MCP](MCP-REMOTES.md). Formato de perfil: [perfiles](MCP-PROFILES.md). Ningún proveedor está certificado en este build.

`status --json` agrega `data.remote` ([JSON Schema](schema/remote-status-v1.json)) y lo calcula sin red, desde la última observación guardada:

- `freshness`: `stored` para el estado guardado, `refreshed` con `--refresh`, `unknown` sin observación.
- `state`: `not_configured`, `synced`, `local_ahead`, `remote_ahead`, `diverged`, `conflicted` o `unknown`, con `reason` cuando es `unknown`.
- `to_publish`, `to_incorporate`, `conflicts`: rutas.
- `base_version_id`, `local_version_id`, `observed_version_id`: identificadores de B, L y R.
- `observed_at`, `age_seconds`, `last_attempt`: antigüedad y completitud de la observación.
- `unmanaged_remote`: elementos remotos que kn no administra.
- `uncommitted_local_changes`: cambios sin guardar de la principal.
- `open_publications`: publicaciones sin verificar.
- `publisher`: `none`, `desktop_client` o `kn`.

Los campos anteriores de `status` se conservan:

- `pending_sync` es `to_publish`.
- `last_remote_observed`, `remote_freshness` y `sync_baseline_id` repiten los datos de `remote`.
- `capabilities` indica qué permite el modo actual con su remoto. No indica permisos. `certified_providers` sigue vacío.

`status --porcelain` no incluye campos remotos: sigue siendo el estado Git local.

Los datos de `fetch` y `push` usan los mismos nombres de estado y rutas. `push --dry-run` devuelve `plan`: operaciones `create_folder`, `create`, `update` o `delete` con su ruta, más `requires_allow_deletes`. `push` devuelve el diario verificado con `published`, `operation_id` y `base_version_id`. `diff --remote` usa `git diff R HEAD`: `added` existe localmente y no en el remoto.

Credenciales: una aplicación integradora entrega el token del servidor MCP en `KN_MCP_ACCESS_TOKEN`. Sin esa variable, kn usa la credencial que guardó `kn remote login` en el almacén del sistema y la renueva si está por vencer. Nunca las recibe en argumentos ni archivos. `remote login` imprime la URL de autorización en stderr e intenta abrir el navegador.

## Estado de integración

`inspect --path <ruta> --json` es experimental y permanece oculto en la ayuda general. Puede devolver `managed=false`, o identificar documentos y su principal; su origen cloud y sharing son desconocidos. `origin.connection` vale `configured` cuando el modo tiene un remoto activo; eso no prueba conectividad. No es un clasificador universal ni sustituye las consultas anteriores. `workspace_id` identifica el historial local, no una cuenta, una persona ni un espacio cloud compartido.

Una aplicación puede consultar kn como proceso; `kn-core` también es una biblioteca Rust, sin estabilidad ABI prometida. Los aliases, envelope y formatos tienen regresiones en [workflow.rs](../crates/kn/tests/workflow.rs). Cada consumidor debe validar su integración contra este build. No inferir integración terminada, permisos cloud o sincronización a partir de un icono.
