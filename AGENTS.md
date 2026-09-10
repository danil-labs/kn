# Trabajar en kn

CLI Rust para documentos con sesiones de agentes. Las carpetas de código usan Git directamente; kn no las administra automáticamente.

## Comandos desde la raíz

```sh
cargo build --workspace --locked
cargo fmt --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

Requiere Git en PATH y Rust compatible con Cargo.toml. Cargo puede usar un target-dir externo: no supongas que el binario está en ./target. Las pruebas usan carpetas temporales y KN_HOME aislado; nunca pruebes restauraciones con documentos reales.

## Mapa

- `crates/kn/src/main.rs`: clap, alias, salida humana/JSON/porcelain y códigos de salida.
- `crates/kn-core/src/git.rs`: único runner Git; conserva el aislamiento del entorno.
- `crates/kn-core/src/workspace.rs`: identidad, descubrimiento, rutas, locks y persistencia.
- `crates/kn-core/src/ops.rs`: cambios, commits, historial, observación y restauración.
- `crates/kn-core/src/sessions.rs`: worktrees e integración a la principal.
- `crates/kn-core/src/plumbing.rs`: consultas para herramientas; `inspect.rs` es experimental.
- `crates/kn-core/src/remote/`: remotos MCP. `http.rs` y `mcp.rs` (transporte), `profile.rs` y `driver.rs` (perfil y operaciones), `oauth.rs` y `credentials.rs` (autorización), `config.rs` (modos), `observe.rs`, `state.rs` y `publish.rs` (observación, B/L/R, publicación).
- `crates/kn/tests/workflow.rs`: recorridos y regresiones con procesos reales.
- `crates/kn/tests/support/mod.rs`: servidor MCP de referencia con fallos inyectables; `remote.rs` y `fake_server.rs` lo usan. `crates/kn-core/tests/oauth.rs` prueba la autorización.

## Reglas que protegen documentos

- Git administra objetos, ramas, diferencias y merges. No agregues otro motor de versiones.
- No sincronices KN_HOME ni gitfiles de sesiones con servicios de archivos cloud.
- No uses barridos de borrado, `git clean` ni reset destructivo para restaurar documentos.
- No descartes Result ni ocultes corrupción como «sin cambios» o «no administrado».
- Conserva NUL/bytes en porcelain; no conviertas nombres a líneas o palabras para parsearlos.
- Las personas editan la principal libremente; el agente trabaja en una sesión externa.
- Los cambios externos se registran como observaciones, no como autoría del agente.
- Ninguna capacidad cloud se declara disponible sin implementación y pruebas del proveedor. `certified_providers` queda vacío hasta registrar esas pruebas en `docs/VALIDATION.md`.
- Una escritura remota solo se envía con precondición de revisión o creación exclusiva declarada en el perfil; si falta, la operación no está disponible. Nada se marca publicado sin una observación completa que lo muestre, y una respuesta perdida se verifica, no se repite.
- Las pruebas remotas usan el servidor de referencia y `KN_MCP_ACCESS_TOKEN`; nunca cuentas o proveedores reales ni el keychain de quien las ejecuta.
- Los locks coordinan procesos kn, no editores ni otras máquinas. No prometas transacciones de carpeta.

## Entrega y documentación

Usa una rama de tarea y PR; verifica los checks antes de integrar. Publica o mergea cuando la persona lo autorice. No alteres otros repos para completar un cambio de kn.

Conserva nombres Git donde corresponda, sin prometer argumentos o semánticas que no estén implementados. Los cambios de salida requieren actualizar `docs/CLI.md` y pruebas de consumidores. Escribe documentación y mensajes en español; identificadores de protocolo en inglés.

`README.md` explica el arranque; `ARCHITECTURE.md`, el sistema actual; `INVARIANTS.md`, las garantías comprobables; `CONTRIBUTING.md`, el flujo de contribución; `SECURITY.md`, el alcance de seguridad; `docs/MCP-REMOTES.md` y `docs/MCP-PROFILES.md`, los remotos MCP y sus perfiles; `docs/RUST-AND-COLLABORATION.md`, la evaluación previa. No dupliques estas fuentes en archivos de instrucciones por proveedor.
