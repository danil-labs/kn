# Evidencia de validación

Evaluación local: 2026-09-10, macOS ARM64, Rust 1.96.0. Alcance: núcleo local y contrato CLI; no es validación de proveedores cloud ni del consumidor Terminus.

| Criterio | Resultado comprobado | Evidencia |
| --- | --- | --- |
| Build del workspace | Aprobado | `cargo build --workspace --locked` |
| Formato | Aprobado | `cargo fmt --check` |
| Lints, incluidos tests | Aprobado | `cargo clippy --workspace --all-targets --locked -- -D warnings` |
| Recorridos y regresiones | 20 pruebas aprobadas en macOS; 2.62 s en la ejecución de referencia | [workflow.rs](../crates/kn/tests/workflow.rs) |
| Remotos MCP de punta a punta | 11 recorridos de la CLI real contra el servidor de referencia (modos, fetch, pull, push, conflictos, respuesta perdida, observación incompleta, deriva de esquema); macOS, 2026-09-10 | [remote.rs](../crates/kn/tests/remote.rs) |
| Cliente MCP y servidor de referencia | 11 pruebas: paginación, SSE y JSON, precondiciones, creación exclusiva, respuesta perdida, token inválido, modo de solo lectura, esquema alterado | [fake_server.rs](../crates/kn/tests/fake_server.rs), [support](../crates/kn/tests/support/mod.rs) |
| Autorización OAuth | 13 pruebas con servidor de autorización falso: PKCE verificado del lado del servidor, `resource`, `state` alterado, sin S256, sin registro dinámico, timeout, renovación | [oauth.rs](../crates/kn-core/tests/oauth.rs) |
| Suite completa | 65 pruebas aprobadas en macOS con `cargo test --workspace --locked` | CI Linux/macOS/Windows pendiente en el PR |
| CI Linux/macOS/Windows | Ejecuciones reales y correcciones de portabilidad en PR #3; consultar el resultado del SHA final | [workflow](../.github/workflows/ci.yml) |
| Documentación de arranque y reglas | Revisada con documentar-para-agentes y root-instructions | [AGENTS.md](../AGENTS.md), [README.md](../README.md) |
| Restauración y concurrencia | Regresiones de ignorados, Unicode, locks y sesiones divergentes | [invariantes](../INVARIANTS.md) |
| Sesión antigua contra principal fresh | Rechazada por identidad | `old_sessions_cannot_publish_to_a_reinitialized_primary` |
| Enlaces absolutos dentro de documentos | Rechazados para impedir que una sesión escriba la principal | `absolute_symlinks_cannot_escape_a_new_sessions_isolation` |
| Proveedores reales: Google Drive, OneDrive, SharePoint | **No certificados**: sin servidor MCP seleccionado, perfil revisado ni pruebas con dos cuentas y cliente de escritorio activo/inactivo | [remotos mediante MCP](MCP-REMOTES.md#criterios-de-aceptación-del-issue-6) |
| Login OAuth contra un servidor de autorización real | No comprobado; solo contra el servidor falso de las pruebas | [perfiles](MCP-PROFILES.md#revisar-un-servidor-nuevo) |
| Fallos de disco y escrituras simultáneas de editores externos | No comprobado | [límites](../SECURITY.md) |
| Integración end-to-end de Terminus | No comprobada | [contrato](CLI.md) |
| MSRV 1.89 | Declarado; validación local ejecutada con Rust 1.96, no con 1.89 | [Cargo.toml](../Cargo.toml) |
| Distribución con checksums e importación de Go | Pendiente | [issue #2](https://github.com/soydanil/kn/issues/2) |

Los checks del PR son la evidencia remota del commit revisado. No se considera terminada una integración por tener CI configurada. La propuesta Go permanece como referencia histórica y no es el motor de esta entrega.

La CI Windows detectó que las rutas verbatim de Rust necesitan adaptación al proceso Git. También mostró diferencias en fechas de directorios entre enumeraciones; la regresión de solo lectura compara el conjunto de entradas, bytes y fechas de archivos, sin exigir estabilidad de fechas de directorios. Hay una prueba unitaria adicional para rutas Windows de disco y UNC.
