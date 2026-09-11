# Invariantes verificables

Estas propiedades se comprueban con [workflow.rs](crates/kn/tests/workflow.rs), salvo las dos últimas, que comprueban [cloud.rs](crates/kn-core/src/cloud.rs) y [git.rs](crates/kn-core/src/git.rs). No implican protección contra procesos externos, errores de disco o modificaciones hostiles de KN_HOME.

| Propiedad | Regresión |
| --- | --- |
| El trabajo de sesión no cambia la principal antes de integrar | `documents_sessions_restore_and_publish_locally` |
| Restaurar conserva ignorados y gitfiles; solo elimina documentos versionados pertinentes | `documents_sessions_restore_and_publish_locally`, `restore_does_not_overwrite_an_ignored_obstruction` |
| Los nombres con acentos, espacios y caracteres especiales no se dividen para parsearlos | `special_names_ignores_empty_folders_and_read_only_queries` |
| La configuración Git del entorno no impone firma o conversión de bytes | `global_git_configuration_is_isolated` |
| Las copias no operan silenciosamente sobre el historial de una ubicación todavía existente | `copies_moves_locks_and_invalid_input` |
| Las lecturas no crean marcadores en documentos | `special_names_ignores_empty_folders_and_read_only_queries` |
| Inspect no inicializa carpetas ni modifica documentos o estado | `inspect_unmanaged_folders_does_not_initialize_or_classify_code`, `inspect_is_read_only_even_after_manual_changes_and_a_move` |
| Un enlace externo se reporta, pero no puede registrarse | `unsafe_symlink_is_reported_but_cannot_be_committed` |
| Los cambios manuales son una base para el agente, no un impedimento para iniciar | `manual_documents_become_the_next_agent_sessions_baseline` |
| Los conflictos de persona/agente permanecen en la sesión | `manual_edits_conflict_only_inside_the_agent_session` |
| Una sesión antigua no escribe una principal reinicializada | `old_sessions_cannot_publish_to_a_reinitialized_primary` |
| Los symlinks absolutos no permiten escribir la principal desde una sesión | `absolute_symlinks_cannot_escape_a_new_sessions_isolation` |
| Las sesiones divergentes no sobrescriben la principal | `concurrent_sessions_require_update_and_preserve_main_on_conflict` |
| Las capacidades cloud no implementadas devuelven unsupported | `cloud_capabilities_remain_honest` |
| Los nombres nuevos conservan alias y las consultas separan JSON de bytes | `git_named_queries_and_aliases_support_service_integration` |
| Un documento que sigue en la nube no bloquea el inicio, no entra en la sesión hasta descargarse y no se pisa al integrar | `cloud_only_documents_wait_without_blocking_or_being_deleted` |
| Un init que falla no deja historiales huérfanos en KN_HOME | `a_failed_init_leaves_no_orphan_history` |
| Integrar registra antes lo pendiente de la sesión | `finish_records_pending_edits_before_integrating` |
| KN_GIT gana sobre PATH; si es inválida, falla con GIT_MISSING sin caer a PATH; sin Git, init no crea `.kn/` ni KN_HOME | `git_resolution_follows_kn_git_and_reports_missing_git` |
| `status` informa lo descargado desde la última observación sin escribir el registro, que vive en KN_HOME | `cloud_downloads_are_reported_without_writing` |
| Lo descargado de la nube queda en su propia versión `cloud_download`, separada de los cambios manuales; una sesión no escribe el registro | `cloud_downloads_have_their_own_version` |
| `cloud fetch` lee los pendientes de la principal, respeta `--max-bytes` y no crea versiones | `cloud_fetch_reads_pending_documents_without_versions` |
| Una lectura que se agota se reporta como `timeout` y no retiene a quien llama | `a_blocked_read_times_out_and_leaves_the_caller_free` |
| Todo proceso se crea con `git::process`, que en Windows usa `CREATE_NO_WINDOW` | `every_process_is_built_through_process` |

Toda corrección de pérdida de datos requiere una regresión con documentos temporales y un control explícito de qué archivos deben sobrevivir. Las limitaciones que no tienen una prueba aparecen en [SECURITY.md](SECURITY.md) y [ARCHITECTURE.md](ARCHITECTURE.md), no se presentan como garantías.
