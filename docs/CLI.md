# Contrato CLI

Fuente de implementación: [main.rs](../crates/kn/src/main.rs), [plumbing.rs](../crates/kn-core/src/plumbing.rs), [error.rs](../crates/kn-core/src/error.rs). Versión del binario: Cargo.toml; versión de envelope JSON: `1.0`. No es una implementación completa de todos los comandos Git.

## Nombres y operaciones

| Comando | Resultado |
| --- | --- |
| `init [--fresh]` | Registra la carpeta en KN_HOME y captura sus documentos, sin escribir en ella. Si ya estaba registrada, o tiene un marcador cuyo historial está en esta máquina, la reutiliza (`already_exists: true`). fresh registra otra identidad; el historial anterior queda sin referencia |
| `migrate` | Registra una principal con `.kn/config.json` y quita su `.kn`; véase [Identidad de una carpeta](#identidad-de-una-carpeta) |
| `status [--refresh]` | Cambios, conflictos, capacidades; refresh no disponible |
| `diff [--patch] [--base\|--remote]` | Cambios desde HEAD; base/remote no disponibles |
| `commit [-m mensaje]` | Registra una versión con los documentos permitidos de la sesión; alias snapshot |
| `log [--limit N] [--offset N]` | Versiones documentales, limit 1–1000, offset desde cero; alias history |
| `restore v_<12 hex>` | Registra antes los cambios y restaura dentro de la sesión |
| `worktree add <nombre>` | Crea sesión bajo KN_HOME; alias session start |
| `worktree list` | Lista sesiones conservadas; con porcelain incluye principal y registros Git |
| `worktree update` | Integra la principal en la sesión actual |
| `worktree finish` | Registra lo pendiente (`recorded_document_count`) e integra la sesión a la principal si puede avanzar fast-forward |
| `cloud fetch [--all] [--timeout-secs N] [--max-bytes N] [--progress] [--retry-failed]` | Descarga los documentos de la principal que siguen en la nube leyéndolos y recuerda los que fallan; no crea versiones |
| `connect [proveedor]`, `pull`, `push` | Capacidad no disponible; no solicitan ni usan tokens |
| `version`, `--version` | Versión del build |

`-C <carpeta>` fija el directorio de ejecución; se admite una sola ocurrencia. No cambia el cwd del consumidor. `--help` funciona por comando. El nombre de sesión acepta 1–64 letras ASCII, números, guiones y guiones bajos; no recibe una ruta arbitraria.

## Entorno

| Variable | Efecto |
| --- | --- |
| `KN_HOME` | Carpeta del historial y las sesiones; `~/.kn` por defecto |
| `KN_GIT` | Ruta absoluta del ejecutable git. Si está definida, gana sobre PATH; si es inválida, el comando falla con GIT_MISSING y no busca en PATH |

Sin `KN_GIT`, kn usa el primer `git` de PATH (`git.exe` en Windows) e ignora las entradas relativas. La elección se hace una vez por proceso. `init` la comprueba antes de crear KN_HOME. En Windows, los procesos que kn lanza no abren ventana de consola.

## Consultas Git para consumidores

`rev-parse` requiere exactamente una consulta:

| Consulta | stdout con terminador LF |
| --- | --- |
| `--is-inside-work-tree` | `true` dentro de kn; fuera de kn, error con stdout vacío |
| `--show-toplevel` | Ruta absoluta de la carpeta kn más cercana que contiene el cwd: principal registrada, principal con marcador o sesión |
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

[JSON Schema del envelope](schema/envelope-v1.json). `data` depende del comando y todavía no tiene un esquema publicado por operación. El mensaje humano es explicativo y no debe parsearse. En errores, data es null; cada error contiene `code`, `message`, `retryable` y `suggested_next_action`. Solo WORKSPACE_BUSY y CLOUD_FETCH_BUSY se marcan retryable.

| Exit | status | Interpretación |
| --- | --- | --- |
| 0 | ok | Operación o ayuda exitosa |
| 1 | error | Error operativo: IO, Git, estado, identidad o bloqueo |
| 2 | conflict | Integración o restauración necesita intervención |
| 3 | unsupported | Entrada inválida, sesión requerida o capacidad no implementada; consultar code |

Los códigos numéricos son de kn, no una reproducción exacta de los de Git. En modo humano o porcelain los errores van a stderr. En JSON, incluso los errores de argumentos llevan envelope. Los programas deben revisar exit y error.code, no solo el texto o status.

| code | Situación |
| --- | --- |
| INVALID_INPUT | Argumentos, formato de versión o schema no admitido; `init` dentro de una sesión o de KN_HOME; un marcador cuyo historial no está en esta máquina; `migrate` desde una sesión |
| UNSUPPORTED_CAPABILITY | Cloud u otra operación no implementada |
| NOT_A_WORKSPACE | No se descubrió una carpeta kn; también, una principal registrada después de moverla |
| WORKSPACE_BUSY | Se agotó la espera de lock |
| CLOUD_FETCH_BUSY | Otra `cloud fetch` del mismo historial está en curso; esta no espera |
| WORKSPACE_COPIED | Un marcador anterior cuya identidad está registrada o ubicada en otra carpeta todavía existente |
| WORKSPACE_IDENTITY_CHANGED | Una sesión cuya principal tiene ahora otra identidad, registrada o en su marcador |
| SESSION_REQUIRED | Escritura de documentos fuera de una sesión |
| CONFLICT | Integración divergente, conflictos o archivos no versionados que obstruyen; `migrate` ante archivos ajenos en `.kn` o un marcador de otro historial |
| UNSAFE_PATH | Registro, symlink o ruta que viola las restricciones |
| GIT_FAILED | Falló el proceso Git |
| GIT_MISSING | No hay Git: KN_GIT es relativa, no existe o no es un archivo, o no está definida y PATH no tiene git. El mensaje nombra el remedio del sistema |
| IO_ERROR | Falló filesystem/proceso |
| INVALID_STATE | Estado JSON no se puede deserializar |

## Identidad de una carpeta

`$KN_HOME/roots.json` registra cada principal por su ruta canónica:

```json
{
  "schema_version": 1,
  "roots": {
    "/Users/ana/Documentos/propuesta": "0b8f2c5e-3d1a-4f7e-9c6b-2a4d8e1f7c30"
  }
}
```

Una raíz tiene una sola identidad, y una identidad una sola raíz. kn escribe el registro con escritura temporal y rename, bajo `$KN_HOME/roots.lock`. Lo actualizan `init`, `migrate` y las operaciones que escriben en el historial de una principal; las consultas, `status`, `diff`, `log` e `inspect` solo lo leen. Un `schema_version` desconocido es INVALID_INPUT; un archivo ilegible, INVALID_STATE.

La carpeta kn es la más cercana al cwd que está registrada o que tiene `.kn/config.json`: con `session`, una sesión; sin `session`, una principal inicializada antes del registro. En la misma carpeta gana el registro. Una carpeta dentro de una principal puede ser otra principal, con su propio historial.

`init` sobre una carpeta con marcador anterior la reutiliza si su historial está en esta máquina. Si no, registra una identidad nueva y no toca el marcador. `init --fresh` siempre registra una identidad nueva; el historial anterior queda intacto en `$KN_HOME/repos/`, sin referencia.

Límite: una principal registrada que se mueve o se renombra no está registrada en su nueva ruta. Allí es NOT_A_WORKSPACE, e `init` empieza otro historial.

### `migrate`

`kn migrate`, desde la principal o una subcarpeta suya, pasa una principal con marcador al registro:

1. Comprueba que el historial del `workspace_id` del marcador esté en esta máquina y que la carpeta no sea una copia (WORKSPACE_COPIED).
2. Registra la raíz.
3. Quita `<raíz>/.kn/config.json`, `<raíz>/.kn/kn.lock` y la carpeta `.kn`, que queda vacía.

Si `.kn` contiene cualquier otro archivo, o un marcador de otro historial, devuelve CONFLICT y no quita nada; el mensaje nombra los archivos. En una carpeta ya migrada no cambia nada y termina con 0.

| Campo | Contenido |
| --- | --- |
| `registered` | `true` si esta ejecución registró la raíz |
| `removed` | Rutas absolutas quitadas, en el orden en que se quitaron |
| `workspace_id` | Identidad de la principal |
| `root` | Raíz canónica |

## Documentos que siguen en la nube

`init`, `status` y `worktree add` devuelven `cloud_only`: las rutas relativas, con `/`, de los documentos que el proveedor muestra pero todavía no descargó, versionados o no. En macOS se reconocen por `SF_DATALESS`; en Windows, por los atributos `OFFLINE`, `RECALL_ON_OPEN` o `RECALL_ON_DATA_ACCESS`. En Linux la lista siempre está vacía.

`status` también devuelve `cloud_only_bytes`, la suma del tamaño lógico de esos documentos, y `cloud_failed`, los de `cloud_only` cuya última descarga falló. El tamaño sale de los metadatos: leerlo no pide la descarga. En una sesión `cloud_failed` siempre está vacía.

En una sesión, `status` devuelve además `primary_cloud_only` y `primary_cloud_only_bytes`: lo mismo para la principal, porque el agente trabaja en la sesión y ahí `cloud_only` queda vacía. Son `null` en la principal y si la principal no se encuentra.

Leer uno de esos documentos obligaría a descargarlo, y sin el cliente de sincronización la lectura se agota. Por eso Git no los lee:

- Uno sin versionar no aparece en `local_changes`, `clean` no lo cuenta y no entra en la versión ni en las sesiones. Cuando el proveedor lo descarga, la siguiente observación lo versiona.
- Uno ya versionado que el proveedor liberó cuenta como sin cambios: su contenido ya está en el historial y las sesiones lo reciben completo desde ahí. Cuando se descarga, Git lo compara: si es igual a su versión no hay cambio; si alguien lo editó en la nube, aparece como `modified`. Límite: mientras siga en la nube, una edición hecha en la nube no se ve.

`worktree finish` nunca los pisa: si la sesión trae un documento en la ruta de uno sin versionar, o cambia uno versionado que sigue en la nube, devuelve CONFLICT sin tocar la principal. Descargarlo con `cloud fetch` y volver a integrar lo resuelve.

`status --porcelain` también trata como sin cambios los versionados que siguen en la nube; no oculta los nuevos, que Git lista sin leerlos.

### Lo descargado desde la última observación

Cada vez que kn registra una versión en la principal (el commit inicial de `init` y cada observación de `worktree add`, `update` o `finish`), guarda la lista `cloud_only` de ese momento en `$KN_HOME/repos/<workspace_id>/cloud-pending.json`, con escritura atómica. Una observación sin cambios también la actualiza. Nunca se escribe en la carpeta de documentos ni desde una sesión.

El registro, en schema 2, guarda también `failed: [{path, reason, at}]`: los documentos cuya descarga falló, con el motivo y la hora UTC en RFC 3339. kn lee también el schema 1, sin `failed`; un kn anterior no lee el schema 2. Toda escritura del registro toma `cloud-pending.lock`, en la misma carpeta.

`status` en la principal devuelve `downloaded_since_last_observation`: las rutas de esa lista que ya no siguen en la nube, existen y Git ve como documentos nuevos, es decir, que todavía no están en HEAD ni ignorados. `status` solo lee el registro. En una sesión la lista siempre está vacía. `worktree add` devuelve la misma lista para la principal: son los documentos que esa observación acaba de versionar.

Al observar, esos documentos se registran primero, solos, en una versión con mensaje «Documentos descargados de la nube» y `Kn-Reason: cloud_download`. El resto de los cambios externos va después en otra versión `external_observation`. Si solo hay un tipo, se crea una sola versión. `log` muestra `reason: "cloud_download"` como las demás razones: `init`, `external_observation`, `manual_snapshot`, `pre_restore_snapshot`, `restore`, `pre_finish_snapshot` y `session_update`.

Límite: la clasificación depende del registro. Un documento que el proveedor descarga antes de que kn lo haya visto en la nube se versiona como `external_observation`.

### `cloud fetch`

`cloud fetch` descarga los documentos de la principal que siguen en la nube. Se puede ejecutar desde la principal o desde una sesión; siempre actúa sobre la principal. Lee cada documento entero, uno por uno, del menor al mayor tamaño aparente, en un hilo con su propio límite de espera.

Sin `--all` hace una pasada sobre lo pendiente al empezar. Con `--all` trabaja por tandas de hasta 32 documentos y vuelve a recorrer la carpeta entre tandas, hasta que no queda nada que intentar: todo lo pendiente ya se leyó o falló. Cada documento se intenta una vez por ejecución.

| Opción | Efecto |
| --- | --- |
| `--timeout-secs N` | Segundos de espera por documento; 60 por defecto, mínimo 1 |
| `--max-bytes N` | Tope total de la ejecución, también con `--all`. Se detiene antes de superar N bytes, sumando el tamaño aparente de cada documento intentado; los restantes van a `skipped_budget`. Con 0 no lee ninguno, tampoco los de tamaño aparente cero: abrirlos ya pide la descarga. Sin la opción no hay tope |
| `--all` | Sigue por tandas hasta que no quede nada que intentar |
| `--progress` | Escribe en stderr una línea JSON por documento intentado; stdout conserva el único envelope |
| `--retry-failed` | Vuelve a intentar los documentos cuya descarga falló antes |

Cada línea de `--progress`, con o sin `--all`, es un objeto:

| Campo | Contenido |
| --- | --- |
| `path` | Documento intentado |
| `outcome` | `fetched` o `failed` |
| `bytes` | Bytes leídos; 0 si falló |
| `fetched_count`, `failed_count` | Totales de esta ejecución |
| `remaining_count`, `bytes_remaining` | Documentos, y su tamaño aparente, que esta ejecución todavía puede intentar según el último recorrido; no cuenta los fallos recordados que omite |
| `bytes_fetched` | Bytes leídos en esta ejecución |

`data` contiene:

| Campo | Contenido |
| --- | --- |
| `fetched` | Rutas leídas completas |
| `failed` | `[{path, reason}]`; `reason` es `timeout` o el texto del error de IO |
| `skipped_budget` | Rutas que no se intentaron por `--max-bytes` |
| `remaining` | Rutas que siguen en la nube al terminar |
| `bytes_fetched` | Bytes leídos de los documentos de `fetched` |

Los fallos se recuerdan en el registro. Las ejecuciones siguientes los omiten, salvo con `--retry-failed`: siguen en `remaining`, y `status` los lista en `cloud_failed`. Un fallo se olvida cuando el documento se descarga, deja de estar en la nube o desaparece.

Dos `cloud fetch` del mismo historial no corren a la vez, tampoco una desde la principal y otra desde una sesión. La primera toma `$KN_HOME/repos/<workspace_id>/cloud-fetch.lock`; la segunda termina enseguida con CLOUD_FETCH_BUSY.

La salida humana es una línea de resumen. Un documento que falla es un resultado: el comando termina con 0. Una carpeta sin kn u otro error operativo usa el envelope de error habitual. `fetch` no crea versiones y del registro solo escribe `failed`: la siguiente observación versiona lo descargado como `cloud_download`. No mantiene el lock del espacio mientras lee, así que otras operaciones de kn pueden correr en paralelo.

Una lectura agotada no se puede cancelar. Su hilo queda suelto y termina con el proceso; el proveedor puede seguir descargando ese documento. Sin verificar: que el proceso salga de inmediato si el sistema mantiene la lectura bloqueada dentro del kernel.

## Estado de integración

`inspect --path <ruta> --json` es experimental y permanece oculto en la ayuda general. Puede devolver `managed=false`, o identificar documentos y su principal; su origen cloud y sharing son desconocidos. No es un clasificador universal ni sustituye las consultas anteriores. `workspace_id` identifica el historial local, no una cuenta, una persona ni un espacio cloud compartido.

Una aplicación puede consultar kn como proceso; `kn-core` también es una biblioteca Rust, sin estabilidad ABI prometida. Los aliases, envelope y formatos tienen regresiones en [workflow.rs](../crates/kn/tests/workflow.rs). Cada consumidor debe validar su integración contra este build. No inferir integración terminada, permisos cloud o sincronización a partir de un icono.
