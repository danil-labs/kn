# Trabajar en kn

CLI Rust para documentos con sesiones de agentes. Las carpetas de código usan Git directamente; kn no las administra automáticamente.

## Comandos desde la raíz

```sh
cargo build --workspace --locked
cargo fmt --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

Requiere Git (el de KN_GIT o, si no está definida, el de PATH) y Rust compatible con Cargo.toml. Cargo puede usar un target-dir externo: no supongas que el binario está en ./target. Las pruebas usan carpetas temporales y KN_HOME aislado; nunca pruebes restauraciones con documentos reales.

## Mapa

- `crates/kn/src/main.rs`: clap, alias, salida humana/JSON/porcelain y códigos de salida.
- `crates/kn-core/src/git.rs`: único runner Git; resuelve el ejecutable y conserva el aislamiento del entorno. Todo proceso se crea con `git::process`, que en Windows no abre consola.
- `crates/kn-core/src/workspace.rs`: identidad, descubrimiento, rutas, locks y persistencia.
- `crates/kn-core/src/ops.rs`: cambios, commits, historial, observación y restauración.
- `crates/kn-core/src/sessions.rs`: worktrees e integración a la principal.
- `crates/kn-core/src/plumbing.rs`: consultas para herramientas; `inspect.rs` es experimental.
- `crates/kn/tests/workflow.rs`: recorridos y regresiones con procesos reales.

## Reglas que protegen documentos

- Git administra objetos, ramas, diferencias y merges. No agregues otro motor de versiones.
- No sincronices KN_HOME ni gitfiles de sesiones con servicios de archivos cloud.
- No uses barridos de borrado, `git clean` ni reset destructivo para restaurar documentos.
- No descartes Result ni ocultes corrupción como «sin cambios» o «no administrado».
- Conserva NUL/bytes en porcelain; no conviertas nombres a líneas o palabras para parsearlos.
- Las personas editan la principal libremente; el agente trabaja en una sesión externa.
- Los cambios externos se registran como observaciones, no como autoría del agente.
- Ninguna capacidad cloud se declara disponible sin implementación y pruebas del proveedor.
- Los locks coordinan procesos kn, no editores ni otras máquinas. No prometas transacciones de carpeta.

## Entrega y documentación

Usa una rama de tarea y PR; verifica los checks antes de integrar. Publica o mergea cuando la persona lo autorice. No alteres otros repos para completar un cambio de kn.

Conserva nombres Git donde corresponda, sin prometer argumentos o semánticas que no estén implementados. Los cambios de salida requieren actualizar `docs/CLI.md` y pruebas de consumidores. Escribe documentación y mensajes en español; identificadores de protocolo en inglés.

`README.md` explica el arranque; `ARCHITECTURE.md`, el sistema actual; `INVARIANTS.md`, las garantías comprobables; `CONTRIBUTING.md`, el flujo de contribución; `SECURITY.md`, el alcance de seguridad; `docs/RUST-AND-COLLABORATION.md`, la evaluación y el diseño cloud pendiente. No dupliques estas fuentes en archivos de instrucciones por proveedor.
