# Rust, sesiones y colaboración

Evaluación: 2026-09-10. Fuentes de producto: solicitud del usuario en esta sesión, incluida la restricción de no sincronizar archivos Git y la pregunta sobre colaboración. Referencia existente: [issue #2](https://github.com/soydanil/kn/issues/2) y [PR #1](https://github.com/soydanil/kn/pull/1), revisado en `2e2189724fc48c5d190793b8e75a6ae0f49cc936`. La propuesta Go permanece intacta en su rama; este documento registra las diferencias, sin reescribir su entrega histórica.

## Aclaración de producto: agentes sobre una carpeta de uso libre

El usuario precisó el 2026-09-10 que las personas pueden compartir y editar documentos manualmente; el objetivo central del versionado es controlar el trabajo de agentes, como en Terminus. Esto corrige la interpretación inicial que exigía una principal limpia para abrir sesiones y que trataba a todo colaborador como usuario de kn.

Las personas no necesitan instalar kn, crear commits ni autenticar su máquina con kn para seguir usando la carpeta. Se conservan los permisos de su servicio y aplicaciones habituales. La autenticación cloud de kn solo será necesaria para que kn acceda al proveedor en nombre de la cuenta que lo ejecuta.

Flujo local implementado en [sessions.rs](../crates/kn-core/src/sessions.rs): antes de abrir/actualizar una sesión o integrarla, se registran los cambios observados en la principal como un commit `external_observation`, sin cambiar el contenido de los documentos. El agente parte de ese estado y guarda sus cambios por separado. Si la principal avanzó, la sesión incorpora la nueva base con Git antes de integrar; un conflicto queda dentro de la sesión. Los ignorados conservan la semántica documentada de exclusión.

`external_observation` identifica una observación hecha por kn, no al autor humano. Un archivo agregado o modificado en disco no permite inferir quién lo editó ni si lo hizo una persona, otro agente o el cliente cloud. La futura procedencia del proveedor se guardará cuando esté disponible. No se exige un historial Git compartido entre todos para este caso de uso. El manifiesto compartido sigue siendo una posibilidad de coordinación, no un requisito para reconocer documentos manuales.

Las regresiones `manual_documents_become_the_next_agent_sessions_baseline`, `manual_changes_during_agent_work_are_preserved_and_integrated` y `manual_edits_conflict_only_inside_the_agent_session` están en [workflow.rs](../crates/kn/tests/workflow.rs). Aún no hay garantía de lectura atómica frente a un editor o cliente cloud escribiendo durante la observación.

## Evaluación de Go

La decisión útil es aprovechar Git para objetos, versiones y diferencias. El lenguaje no arregla una restauración incorrecta, no ofrece locks entre procesos automáticamente y no convierte Drive en un remoto Git. Elegimos Rust para la implementación solicitada; no se han medido mejoras de tamaño, velocidad o fiabilidad atribuibles al lenguaje.

Hallazgos leídos en el PR:

| Evidencia fijada al commit | Resultado | Tratamiento en Rust |
| --- | --- | --- |
| [versions.go:189](https://github.com/soydanil/kn/blob/2e2189724fc48c5d190793b8e75a6ae0f49cc936/internal/engine/versions.go#L189) | Barre todos los archivos ausentes del árbol, incluyendo ignorados y archivos de `.git`; parsea nombres citados como si fueran rutas reales | Restauración mediante checkout de dos árboles de Git, con preversión; regresión con Unicode, ignorados y gitfile |
| [git.go:18](https://github.com/soydanil/kn/blob/2e2189724fc48c5d190793b8e75a6ae0f49cc936/internal/engine/git.go#L18) | Hereda configuración y variables Git del proceso | Un solo runner aislado, sin hooks, firma ni filtros de contenido |
| [ops.go](https://github.com/soydanil/kn/blob/2e2189724fc48c5d190793b8e75a6ae0f49cc936/internal/engine/ops.go#L277) funciones Status y Diff | Escribe marcadores durante consultas | Consultas sin modificar documentos; carpetas vacías informativas |
| [config.go](https://github.com/soydanil/kn/blob/2e2189724fc48c5d190793b8e75a6ae0f49cc936/internal/workspace/config.go#L92) funciones LoadConfig y SaveState | Identidad local mezclada con ubicación absoluta y estado de HEAD duplicado | Descubrimiento de raíz, registro externo de ubicación y HEAD leído de Git |

El flujo de una sola carpeta del PR no expresa las sesiones del producto solicitado. El nuevo núcleo usa `git worktree add`, snapshots como commits y avance de la principal solo cuando es fast-forward. Cuando la principal diverge, la sesión incorpora sus cambios con `git merge`; no hay algoritmo propio de merge. [Semántica oficial de worktrees](https://git-scm.com/docs/git-worktree).

## Implementado y sus límites

`crates/kn-core/src/git.rs` concentra Git; `workspace.rs` mantiene identidad local y lock con espera de dos segundos; `ops.rs` implementa consultas/versiones/restauración; `sessions.rs` organiza sesiones e integración. `crates/kn/src/main.rs` contiene argumentos, mensajes y envelope. `crates/kn/tests/workflow.rs` verifica el recorrido y regresiones con procesos reales.

Los worktrees y el historial quedan en KN_HOME, fuera de la principal. La principal puede ser una carpeta sincronizada como transición, pero el cliente de nube puede publicar estados intermedios durante una integración de varios archivos. El lock local solo coordina procesos kn de esta máquina: no coordina editores, clientes cloud, comandos Git directos ni otras máquinas.

La restauración no barre el sistema de archivos: Git calcula las eliminaciones versionadas. No se usa `git clean` ni reset destructivo. Una operación interrumpida puede dejar cambios parciales recuperables desde versiones previas; todavía falta un diario de recuperación y pruebas de fallos de disco. No declarar atomicidad del flujo completo.

Cambios explícitos frente al contrato del issue:

- `snapshot` y `restore` requieren sesión; la principal admite cambios manuales observados e integración de trabajo de agentes.
- La principal no recibe `.git`; las sesiones externas sí tienen el gitfile estándar de Git.
- Se conserva el formato del envelope 1.0 y de los IDs visibles, pero el layout local usa schema 2 y no importa el historial Go. `--fresh` inicia otro historial, no migra el anterior.
- No se mantienen versiones de carpetas vacías. El commit vacío inicial solo ancla worktrees y no aparece en history. HEAD puede identificar ese ancla en init/status.
- `history` tiene limit/offset. El parche no incluye contenido de archivos sin versionar, aunque aparecen en changes.
- Falta compatibilidad campo por campo con Terminus, esquemas JSON de data por operación, códigos específicos por operación y distribución del issue. No cerrar #2 como resuelto por esta entrega.
- Autor local actual: `kn <kn@local>`. No identifica a personas ni autentica publicaciones. Ese dato debe ampliarse antes de habilitar colaboración.

## Diseño propuesto: carpeta compartida y autenticación

Revisión propuesta del 2026-09-10: [remotos mediante MCP](MCP-REMOTES.md). El usuario solicita evitar registros OAuth propios y ejecutar el acceso remoto desde código; el documento nuevo propone un cliente MCP con perfiles de herramientas. La autorización directa descrita abajo permanece como antecedente de diseño; la nueva vía sigue pendiente de revisión y validación de servidores.

Separar identidad compartida, clon local, persona y autorización:

| Dato | Ubicación propuesta | Significado |
| --- | --- | --- |
| `space_id` | manifiesto compartido `.kn-space.json` | La misma carpeta lógica para todos los colaboradores |
| `clone_id`, registro de sesiones | almacenamiento local externo | Esta copia y su historial; no se comparte como identidad del espacio |
| Nombre/correo del autor | configuración personal local | Atribución declarada en Git; no prueba quién tuvo acceso |
| Cuenta del proveedor y token | bóveda del sistema o token efímero del proceso | Permiso efectivo de leer/publicar |
| `publication_id`, commit y revisiones observadas | manifiesto y recibo local | Qué versión se intentó/publicó y contra qué estado remoto |

El manifiesto aún no está implementado. Es un formato versionado de datos sin secretos, objetos Git, rutas locales ni comandos ejecutables. Un nombre sugerido es `.kn-space.json`; el adaptador debe usar su identidad de archivo del proveedor, no confiar exclusivamente en un nombre que podría duplicarse. Su contenido es entrada no confiable y no concede permisos. Un hash de commit es una referencia, no transporta objetos ni autentica al autor. La firma de commits podría autenticar procedencia con una política de claves; tampoco sustituye permisos del proveedor. [Datos de autor y firma en Git](https://git-scm.com/docs/git-commit).

Cada persona que ejecute kn con acceso cloud autoriza su propia cuenta mediante OAuth; quienes solo colaboran manualmente no necesitan autenticar kn. En ejecución integrada, conservar la idea del issue: `KN_PROVIDER`, `KN_ACCOUNT_REF`, `KN_ACCESS_TOKEN`, `KN_ROOT_ID` en el entorno, sin prompts ni credenciales en argumentos/manifiestos/logs. La autenticación humana puede almacenarse en el almacén seguro del sistema. Un identificador de dispositivo sirve para diagnóstico, no como sustituto de autenticación. La autorización y alcance deben validarse con el proveedor. [Scopes de Drive](https://developers.google.com/workspace/drive/api/guides/api-specific-auth).

El caso central es usar historial local para sesiones de agentes sobre documentos compartidos. Para la fase cloud se distinguen dos alcances, todavía sin implementar:

1. **Solo Drive/SharePoint:** todos comparten documentos y revisiones cloud. Cada clon usa Git localmente. Al importar una publicación puede producir otro commit; no se promete un grafo Git idéntico ni un historial completo compartido.
2. **Historial compartido:** remoto Git separado para compartir commits con el protocolo de Git, más Drive/SharePoint como destino documental. No almacenar un repositorio Git vivo dentro de Drive. El enlace al commit en el manifiesto es útil solo cuando los objetos están disponibles en ese remoto. Una exportación Git inmutable para respaldos sería otro transporte a evaluar, no un directorio `.git` sincronizado.

## Publicación controlada pendiente

`push` deberá tomar un commit fijo de main, no leer archivos cambiantes de la carpeta principal. Se construye una lista explícita desde el árbol Git excluyendo metadatos internos y symlinks; nunca copiar recursivamente KN_HOME. Los remotos Git y los documentales son tipos distintos de destino, con capacidades y permisos propios.

Antes de publicar se observa el remoto, se calcula un plan y se comparan revisiones/ETags por elemento. Aplicar escrituras condicionales donde exista soporte real. SharePoint/Graph documenta `if-match` para actualización de metadatos, pero hay que comprobar también subida, eliminación, creación y cada endpoint elegido: no extrapolar esa garantía a toda la carpeta. [Graph update](https://learn.microsoft.com/en-us/graph/api/driveitem-update?view=graph-rest-1.0). Drive ofrece cargas de archivos y seguimiento de cambios, no una transacción Git de múltiples documentos. [Cargas](https://developers.google.com/workspace/drive/api/guides/manage-uploads), [cambios](https://developers.google.com/workspace/drive/api/guides/manage-changes).

No basta con comparar un manifiesto y subir después: existe una carrera entre la lectura y la escritura. Un archivo de lock sincronizado tampoco ofrece exclusión fiable. Si el proveedor no permite la condición necesaria, declarar esa capacidad no disponible o usar un coordinador; no prometer ausencia de sobrescrituras. Registrar operación y resultados por archivo para reintentos idempotentes. La publicación solo se marca completa al verificar todos los resultados; un manifiesto actualizado al final no vuelve atómica la carpeta.

Los colaboradores que editan directamente en Drive no crean commits kn. `pull` deberá reconocer esos documentos como estado externo observado y actualizar la base de las sesiones, conservando procedencia del proveedor cuando esté disponible. No exigirá que el colaborador manual haga un commit o pase por una sesión kn. Los cambios concurrentes se revisan antes de integrar; no atribuir una modificación externa al usuario local. Documentos nativos de Google y archivos binarios necesitan política explícita de exportación, pérdida de información y resolución: fuera del núcleo local.

Siguiente entrega: implementar identidad personal/local separada del espacio para quienes ejecutan agentes, escoger un proveedor y probar sus escrituras condicionales con dos cuentas y fallos intermedios antes de habilitar push.

La evidencia vigente de verificación local y remota se registra en [VALIDATION.md](VALIDATION.md). CI configurada para Linux/macOS/Windows. No se midieron proveedores cloud, cortes de energía, fallos de disco ni compatibilidad binaria con Terminus.

La interfaz de consulta para consumidores usa los nombres `rev-parse`, `status --porcelain` y `worktree list --porcelain`, implementados en [plumbing.rs](../crates/kn-core/src/plumbing.rs). El [README](../README.md#consultas-para-herramientas) define las opciones admitidas, el formato de salida y las diferencias de semántica con Git. La consulta `inspect` queda experimental y oculta; Terminus sigue siendo responsable de clasificar carpetas ajenas a kn. `commit`/`log`/`worktree add` conservan los nombres anteriores como alias.
