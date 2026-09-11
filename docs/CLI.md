# Contrato CLI

Fuente de implementación: [main.rs](../crates/kn/src/main.rs), [plumbing.rs](../crates/kn-core/src/plumbing.rs), [error.rs](../crates/kn-core/src/error.rs). Versión del binario: Cargo.toml; versión de envelope JSON: `1.0`. No es una implementación completa de todos los comandos Git.

## Nombres y operaciones

| Comando | Resultado |
| --- | --- |
| `init [--fresh]` | Inicializa; fresh asigna otra identidad, no convierte el historial anterior |
| `status [--refresh]` | Cambios, conflictos, capacidades; refresh no disponible |
| `diff [--patch] [--base\|--remote]` | Cambios desde HEAD; base/remote no disponibles |
| `commit [-m mensaje]` | Registra una versión con los documentos permitidos de la sesión; alias snapshot |
| `log [--limit N] [--offset N]` | Versiones documentales, limit 1–1000, offset desde cero; alias history |
| `restore v_<12 hex>` | Registra antes los cambios y restaura dentro de la sesión |
| `worktree add <nombre>` | Crea sesión bajo KN_HOME; alias session start |
| `worktree list` | Lista sesiones conservadas; con porcelain incluye principal y registros Git |
| `worktree update` | Integra la principal en la sesión actual |
| `worktree finish` | Registra lo pendiente (`recorded_document_count`) e integra la sesión a la principal si puede avanzar fast-forward |
| `cloud fetch [--timeout-secs N] [--max-bytes N]` | Descarga los documentos de la principal que siguen en la nube leyéndolos; no crea versiones |
| `connect [proveedor]`, `pull`, `push` | Capacidad no disponible; no solicitan ni usan tokens |
| `version`, `--version` | Versión del build |

`-C <carpeta>` fija el directorio de ejecución; se admite una sola ocurrencia. No cambia el cwd del consumidor. `--help` funciona por comando. El nombre de sesión acepta 1–64 letras ASCII, números, guiones y guiones bajos; no recibe una ruta arbitraria.

## Entorno

| Variable | Efecto |
| --- | --- |
| `KN_HOME` | Carpeta del historial y las sesiones; `~/.kn` por defecto |
| `KN_GIT` | Ruta absoluta del ejecutable git. Si está definida, gana sobre PATH; si es inválida, el comando falla con GIT_MISSING y no busca en PATH |

Sin `KN_GIT`, kn usa el primer `git` de PATH (`git.exe` en Windows) e ignora las entradas relativas. La elección se hace una vez por proceso. `init` la comprueba antes de crear `.kn/` o KN_HOME. En Windows, los procesos que kn lanza no abren ventana de consola.

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

[JSON Schema del envelope](schema/envelope-v1.json). `data` depende del comando y todavía no tiene un esquema publicado por operación. El mensaje humano es explicativo y no debe parsearse. En errores, data es null; cada error contiene `code`, `message`, `retryable` y `suggested_next_action`. Solo WORKSPACE_BUSY se marca retryable actualmente.

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
| GIT_MISSING | No hay Git: KN_GIT es relativa, no existe o no es un archivo, o no está definida y PATH no tiene git. El mensaje nombra el remedio del sistema |
| IO_ERROR | Falló filesystem/proceso |
| INVALID_STATE | Estado JSON no se puede deserializar |

## Documentos que siguen en la nube

`init`, `status` y `worktree add` devuelven `cloud_only`: las rutas relativas, con `/`, de los documentos que el proveedor muestra pero todavía no descargó. En macOS se reconocen por `SF_DATALESS`; en Windows, por los atributos `OFFLINE`, `RECALL_ON_OPEN` o `RECALL_ON_DATA_ACCESS`. En Linux la lista siempre está vacía.

Leer uno de esos documentos obligaría a descargarlo, y sin el cliente de sincronización la lectura se agota. Por eso Git no los lista ni los guarda: no aparecen en `local_changes`, `clean` no los cuenta y no entran en la versión ni en las sesiones. Cuando el proveedor los descarga, la siguiente observación los versiona. `worktree finish` nunca los pisa: si la sesión trae un documento en la misma ruta, devuelve CONFLICT.

Límite sin prueba: un documento ya versionado que el proveedor reemplaza por otra versión sin descargarla todavía se lee al observarlo.

### Lo descargado desde la última observación

Cada vez que kn registra una versión en la principal (el commit inicial de `init` y cada observación de `worktree add`, `update` o `finish`), guarda la lista `cloud_only` de ese momento en `$KN_HOME/repos/<workspace_id>/cloud-pending.json`, con escritura atómica. Una observación sin cambios también la actualiza. Nunca se escribe en la carpeta de documentos ni desde una sesión.

`status` en la principal devuelve `downloaded_since_last_observation`: las rutas de esa lista que ya no siguen en la nube, existen y Git ve como documentos nuevos, es decir, que todavía no están en HEAD ni ignorados. `status` solo lee el registro. En una sesión la lista siempre está vacía. `worktree add` devuelve la misma lista para la principal: son los documentos que esa observación acaba de versionar.

Al observar, esos documentos se registran primero, solos, en una versión con mensaje «Documentos descargados de la nube» y `Kn-Reason: cloud_download`. El resto de los cambios externos va después en otra versión `external_observation`. Si solo hay un tipo, se crea una sola versión. `log` muestra `reason: "cloud_download"` como las demás razones: `init`, `external_observation`, `manual_snapshot`, `pre_restore_snapshot`, `restore`, `pre_finish_snapshot` y `session_update`.

Límite: la clasificación depende del registro. Un documento que el proveedor descarga antes de que kn lo haya visto en la nube se versiona como `external_observation`.

### `cloud fetch`

`cloud fetch` descarga los documentos de la principal que siguen en la nube. Se puede ejecutar desde la principal o desde una sesión; siempre actúa sobre la principal. Lee cada documento entero, uno por uno, del menor al mayor tamaño aparente, en un hilo con su propio límite de espera.

| Opción | Efecto |
| --- | --- |
| `--timeout-secs N` | Segundos de espera por documento; 60 por defecto, mínimo 1 |
| `--max-bytes N` | Se detiene antes de superar N bytes, sumando el tamaño aparente de cada documento intentado; los restantes van a `skipped_budget` |

`data` contiene:

| Campo | Contenido |
| --- | --- |
| `fetched` | Rutas leídas completas |
| `failed` | `[{path, reason}]`; `reason` es `timeout` o el texto del error de IO |
| `skipped_budget` | Rutas que no se intentaron por `--max-bytes` |
| `remaining` | Rutas que siguen en la nube al terminar |
| `bytes_fetched` | Bytes leídos de los documentos de `fetched` |

La salida humana es una línea de resumen. Un documento que falla es un resultado: el comando termina con 0. Una carpeta sin kn u otro error operativo usa el envelope de error habitual. `fetch` no crea versiones ni actualiza el registro: la siguiente observación versiona lo descargado como `cloud_download`. No mantiene el lock de kn mientras lee, así que otras operaciones de kn pueden correr en paralelo.

Una lectura agotada no se puede cancelar. Su hilo queda suelto y termina con el proceso; el proveedor puede seguir descargando ese documento. Sin verificar: que el proceso salga de inmediato si el sistema mantiene la lectura bloqueada dentro del kernel.

## Estado de integración

`inspect --path <ruta> --json` es experimental y permanece oculto en la ayuda general. Puede devolver `managed=false`, o identificar documentos y su principal; su origen cloud y sharing son desconocidos. No es un clasificador universal ni sustituye las consultas anteriores. `workspace_id` identifica el historial local, no una cuenta, una persona ni un espacio cloud compartido.

Una aplicación puede consultar kn como proceso; `kn-core` también es una biblioteca Rust, sin estabilidad ABI prometida. Los aliases, envelope y formatos tienen regresiones en [workflow.rs](../crates/kn/tests/workflow.rs). Cada consumidor debe validar su integración contra este build. No inferir integración terminada, permisos cloud o sincronización a partir de un icono.
