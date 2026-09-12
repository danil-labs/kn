# kn

CLI en Rust para trabajar con agentes sobre carpetas de documentos compartidas. Las personas pueden agregar, editar o borrar documentos con sus aplicaciones habituales; los agentes trabajan en sesiones separadas con historial Git. Implementación local experimental, sin proveedores cloud todavía.

```sh
cargo install --path crates/kn --locked
cd /ruta/a/documentos
kn init
kn worktree add propuesta
# Abre la carpeta que imprime el comando y edita ahí.
cd /ruta/impresa/de/la/sesion
kn status
kn diff --patch
kn commit -m "Propuesta revisada"
kn worktree finish
```

`finish` registra lo pendiente, lo integra a la principal y conserva la sesión. Si otra sesión ya avanzó la principal, ejecuta `kn worktree update`, revisa el resultado y vuelve a integrar. Los conflictos permanecen en la sesión; su resolución avanzada usa Git. Al abrir o actualizar una sesión y antes de integrar, `kn` registra los cambios manuales observados en la principal como una versión de referencia (`external_observation`), sin reescribir sus documentos ni atribuir esos cambios al agente. Si la principal avanzó durante el trabajo, se pide actualizar la sesión antes de integrar. `status` solo informa: no crea versiones. No se necesita usar `kn` para colaborar manualmente.

Los documentos que el proveedor muestra pero no descargó quedan fuera del historial (`cloud_only`). `kn cloud fetch` los descarga leyéndolos, sin crear versiones. Cuando se descargan, `status` los lista en `downloaded_since_last_observation`, y la siguiente observación los registra en su propia versión `cloud_download`, separada de los cambios manuales.

`kn log --limit 20 --offset 0` lista versiones. `kn restore v_<12-hex>` restaura dentro de una sesión, registra antes los cambios pendientes y crea una versión solo si hay cambios. No modifica la principal ni envía nada a la nube. `kn worktree list` muestra las sesiones conservadas.

Los comandos de aplicación aceptan `--json`, incluidos errores de argumentos y ayuda. Las consultas `rev-parse` y los modos `--porcelain` usan salida de texto/bytes y rechazan combinarlos con `--json`. Salidas: 0 éxito, 1 error operativo, 2 conflicto, 3 entrada inválida o capacidad no disponible. `connect`, `push`, `pull`, `status --refresh` y `diff --base/--remote` devuelven capacidad no disponible.

## Probar el flujo de sesiones

Usa primero una carpeta desechable fuera de Drive, con dos archivos de texto. Sigue los comandos del arranque: inicializa la principal y crea una sesión. Abre **la ruta de sesión que devuelve kn** en una terminal, editor o IDE. Si usas un agente, arráncalo con esa carpeta como directorio de trabajo. Crear una sesión en otra herramienta no crea automáticamente un worktree kn: esa herramienta debe invocar kn y usar la ruta devuelta. También puedes ejecutar todo el flujo manualmente desde una terminal.

Comprueba estas etapas antes de usar documentos reales:

1. Edita un archivo dentro de la sesión: el documento de la principal debe seguir igual.
2. Revisa `kn status` y `kn diff --patch`, y ejecuta `kn commit -m "Propuesta revisada"` desde la sesión: la principal debe seguir igual. Revisa también los archivos nuevos; el patch no muestra su contenido mientras no estén versionados.
3. Cuando se autorice incorporar la propuesta, ejecuta `kn worktree finish` desde la sesión: ahora la principal recibe los cambios. La sesión se conserva.
4. Para probar colaboración, crea otra sesión, registra un cambio suyo y edita manualmente otro archivo en la principal. Ejecuta `kn worktree update` desde la sesión limpia, revisa la integración y luego `kn worktree finish`. Si hay conflictos, resuélvelos en la sesión y registra el resultado antes de integrar.

Cada sesión usa una rama `sessions/<nombre>` y archivos de trabajo separados; la principal usa `main`. Comparten los objetos y el historial Git local. No es una copia independiente del repositorio ni incluye archivos ignorados o carpetas vacías. Esta separación no es un sandbox: un agente con permisos sobre la principal todavía puede escribir en ella mediante otra ruta. El consumidor debe aplicar los permisos que necesite; véase [seguridad](SECURITY.md).

Después repite con una carpeta desechable sincronizada con Drive, usando archivos descargados y disponibles localmente y manteniendo KN_HOME fuera de cualquier sincronización. Al integrar, el cliente de Drive puede subir los cambios de la principal por su cuenta, archivo por archivo. Esto no equivale a `kn push` ni garantiza una publicación indivisible. Los accesos a documentos nativos de Google no respaldan su contenido mediante este flujo de archivos.

Los colaboradores pueden editar la principal sin usar kn. Compartir esa carpeta por Drive no comparte las ramas ni el historial local. kn no escribe nada en la carpeta: su identidad vive en el KN_HOME de cada máquina y no autentica a personas ni máquinas. El diseño de colaboración cloud pendiente se describe en [la evaluación y propuesta](docs/RUST-AND-COLLABORATION.md). Para integrar kn en otras herramientas y consultar estados, usa el [contrato CLI](docs/CLI.md).

## Dónde viven los archivos

- Principal: solo documentos. kn no crea en ella `.git`, `.kn` ni locks.
- Registro: `$KN_HOME/roots.json`, con la ruta de cada principal y su id.
- Historial: `$KN_HOME/repos/<id>/`, o `~/.kn/repos/<id>/` por defecto.
- Sesiones: `$KN_HOME/sessions/<id>/<nombre>/`. Son worktrees Git reales, con su archivo `.git` local.

**KN_HOME debe quedar fuera de cualquier carpeta sincronizada por Drive, OneDrive u otro cliente.** Se rechaza ubicarlo dentro de la principal, pero no se detectan todas las carpetas sincronizadas del sistema. No se deben sincronizar índices, locks, objetos ni registros de worktrees por un cliente de archivos. Un `.git` existente del usuario se conserva y se señala en `status`.

Copiar los documentos no copia el historial ni la identidad: la copia no es una carpeta kn hasta que se ejecuta `kn init` en ella. Mover o renombrar una principal también la deja sin identidad: `kn init` en la nueva ubicación empieza otro historial, y el anterior queda en KN_HOME. Una carpeta dentro de otra se puede inicializar: cada una tiene su historial, y la de arriba ve lo que integra la de abajo como un cambio externo.

Las carpetas inicializadas con versiones anteriores tienen `.kn/config.json` y siguen funcionando. Una copia con el mismo marcador se rechaza mientras exista la original, y `kn init --fresh` le da historial propio sin reescribir el marcador. `kn migrate`, desde la principal, pasa la identidad al registro y quita `.kn`.

Para respaldar, conserva principal y KN_HOME sin operaciones activas; los registros de worktrees usan rutas absolutas y requieren reparación con Git si se trasladan a otra máquina. Esto todavía no es un mecanismo de colaboración.

`.gitignore` se versiona y Git aplica sus exclusiones. Archivos ignorados no tienen respaldo en el historial. También se excluyen `.kn/` anidadas, basura del sistema y temporales de Office. Las carpetas vacías se reportan, pero **no se versionan**: no se crean marcadores. Los archivos se preservan como bytes, sin filtros ni conversión de finales de línea. Git puede combinar texto; documentos binarios en conflicto requieren elegir o editar una versión con su aplicación.

## Qué Git usa kn

kn elige el ejecutable una vez por proceso, en este orden:

1. `KN_GIT`, si está definida: la ruta absoluta de un ejecutable git. Si la ruta es relativa, no existe o no es un archivo, kn falla con `GIT_MISSING`; no busca en PATH.
2. El primer `git` de PATH (`git.exe` en Windows). Las entradas relativas de PATH se ignoran.
3. Si no hay ninguno, falla con `GIT_MISSING` y un remedio por sistema: `xcode-select --install` en macOS, el gestor de paquetes en Linux, Git for Windows en Windows; en todos, definir KN_GIT.

`init` comprueba Git antes de crear KN_HOME. kn no instala Git. Una aplicación que empaqueta su propio git, como Terminus, lo pasa en KN_GIT.

## Usar kn-core como librería

`kn` (`crates/kn`) es un cascarón de línea de comandos sobre `kn-core`
(`crates/kn-core`): parsea argumentos con clap y nada más. `kn-core` no
depende de ese proceso — no imprime, no llama `process::exit` ni lee
argumentos de la línea de comandos —, así que una aplicación Rust que ya
administra sus propias sesiones puede depender de `kn-core` directamente y
llamar sus funciones (`workspace::Workspace::open`, `ops::status`,
`ops::diff`, `ops::commit`, `sessions::*`) en su propio proceso, sin lanzar
`kn` como subproceso ni parsear su salida `--json`. El `Envelope` de
[error.rs](crates/kn-core/src/error.rs) es el mismo contrato, ya como valor
Rust en vez de texto.

Quien embeba `kn-core` hereda la resolución de Git de la sección anterior tal
cual es: una sola vez por proceso. Hay que fijar `KN_GIT` antes de la primera
llamada a `kn-core`, no antes de cada una — una aplicación que empaqueta su
propio git, como Terminus, la define una vez al arrancar.

Esto no cambia el contrato del CLI ni resuelve la distribución de `kn` como
binario aparte para quien sí necesita invocarlo como programa externo (véase
el issue #14): es la otra vía, para quien construye directamente sobre
`kn-core` en Rust.

## Alcance y verificación

Requiere Rust 1.89+ y Git (véase la sección anterior). Se verifica localmente con:

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
```

Consulta también [arquitectura](ARCHITECTURE.md), [contrato CLI](docs/CLI.md), [invariantes](INVARIANTS.md), [contribución](CONTRIBUTING.md), [seguridad](SECURITY.md), [changelog](CHANGELOG.md) e [instrucciones para agentes](AGENTS.md).

La [revisión de la propuesta](docs/RUST-AND-COLLABORATION.md) explica los cambios respecto al PR Go, el diseño cloud y las incompatibilidades pendientes. No hay importación del historial Go, autenticación, manifiesto compartido implementado, remoto Git administrado, releases automáticos ni garantía transaccional ante cierres a mitad de una operación. La compatibilidad de cada herramienta consumidora debe validarse contra el contrato CLI.

## Consultas para herramientas

```sh
kn -C /ruta/documentos rev-parse --is-inside-work-tree
kn -C /ruta/documentos rev-parse --show-toplevel
kn -C /ruta/documentos rev-parse --git-dir
kn -C /ruta/documentos rev-parse --git-common-dir
kn -C /ruta/documentos rev-parse HEAD
kn -C /ruta/documentos status --porcelain -z
kn -C /ruta/documentos worktree list --porcelain -z
```

Las consultas son de solo lectura y requieren una carpeta kn válida. No inicializan carpetas de código ni clasifican carpetas ajenas. `rev-parse` admite una consulta por invocación: imprime `true`, una ruta absoluta o el SHA completo, seguido de salto de línea. Fuera de kn termina con código 1 y stdout vacío; un error operativo tampoco debe interpretarse automáticamente como carpeta de código. Las consultas no verifican conectividad cloud ni permisos compartidos.

`status --porcelain` usa el formato v1 de Git directamente. `worktree list --porcelain` conserva los registros de Git, ajustando únicamente la ruta de la principal: Git reporta el directorio de metadatos externo, mientras kn conoce la carpeta documental. `-z` conserva nombres con espacios, acentos y saltos de línea sin ambigüedad; úsalo en integraciones. Las rutas Git expuestas son locales y nunca deben subirse a Drive. Referencias de formato: [status](https://git-scm.com/docs/git-status), [worktree](https://git-scm.com/docs/git-worktree), [rev-parse](https://git-scm.com/docs/git-rev-parse).

`commit`, `log` y `worktree add` son los nombres preferidos. `snapshot`, `history` y `session start` siguen como alias. No se promete compatibilidad completa con todos los argumentos o semánticas de Git: `commit` registra una versión con todos los cambios de documentos permitidos en la sesión; `worktree add <nombre>` elige una ruta segura bajo KN_HOME, no recibe una ruta arbitraria. `worktree update` y `worktree finish` conservan el flujo específico de integrar sesiones de kn. `inspect` se conserva oculto como consulta JSON experimental, no es el contrato recomendado de integración.

## Licencia

kn se distribuye bajo cualquiera de estas dos licencias, a elección de quien lo use:

- [Apache License 2.0](LICENSE-APACHE)
- [MIT](LICENSE-MIT)

Salvo que se indique lo contrario, toda contribución enviada para incluirse en kn se licencia de la misma forma, sin términos adicionales.
