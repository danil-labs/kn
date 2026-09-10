# Invariantes verificables

Estas propiedades se comprueban con [workflow.rs](crates/kn/tests/workflow.rs), [remote.rs](crates/kn/tests/remote.rs), [fake_server.rs](crates/kn/tests/fake_server.rs) y [oauth.rs](crates/kn-core/tests/oauth.rs). Las pruebas remotas usan el servidor MCP de referencia de [support](crates/kn/tests/support/mod.rs), no proveedores reales. No implican protección contra procesos externos, errores de disco o modificaciones hostiles de KN_HOME.

| Propiedad | Regresión |
| --- | --- |
| El trabajo de sesión no cambia la principal antes de integrar | `documents_sessions_restore_and_publish_locally` |
| Restaurar conserva ignorados y gitfiles; solo elimina documentos versionados pertinentes | `documents_sessions_restore_and_publish_locally`, `restore_does_not_overwrite_an_ignored_obstruction` |
| Los nombres con acentos, espacios y caracteres especiales no se dividen para parsearlos | `special_names_ignores_empty_folders_and_read_only_queries` |
| La configuración Git del entorno no impone firma o conversión de bytes | `global_git_configuration_is_isolated` |
| Las copias no operan silenciosamente sobre el historial de una ubicación todavía existente | `copies_moves_locks_and_invalid_input` |
| Las lecturas no crean marcadores en documentos | `special_names_ignores_empty_folders_and_read_only_queries` |
| Inspect no inicializa carpetas ni modifica documentos o estado | `inspect_unmanaged_folders_does_not_initialize_or_classify_code`, `inspect_is_read_only_even_after_manual_changes_and_a_move` |
| Un enlace externo se reporta, pero no puede guardarse | `unsafe_symlink_is_reported_but_cannot_be_committed` |
| Los cambios manuales son una base para el agente, no un impedimento para iniciar | `manual_documents_become_the_next_agent_sessions_baseline` |
| Los conflictos de persona/agente permanecen en la sesión | `manual_edits_conflict_only_inside_the_agent_session` |
| Una sesión antigua no escribe una principal reinicializada | `old_sessions_cannot_publish_to_a_reinitialized_primary` |
| Los symlinks absolutos no permiten escribir la principal desde una sesión | `absolute_symlinks_cannot_escape_a_new_sessions_isolation` |
| Las sesiones divergentes no sobrescriben la principal | `concurrent_sessions_require_update_and_preserve_main_on_conflict` |
| Sin un modo explícito, los comandos cloud no contactan servidores; ningún proveedor se declara certificado | `cloud_capabilities_remain_honest`, `cloud_commands_require_an_explicit_transfer_mode` |
| Los nombres nuevos conservan alias y las consultas separan JSON de bytes | `git_named_queries_and_aliases_support_service_integration` |
| Observar el remoto no cambia la principal ni su historial | `fetch_imports_the_complete_remote_without_touching_main` |
| En `desktop_sync_observed` ninguna escritura llega al servidor; push se rechaza antes de contactarlo | `observed_desktop_mode_integrates_locally_and_never_writes_through_mcp`, `read_only_mode_never_sends_writes` |
| Sin base reconciliada no se elige ganador; los conflictos quedan en la sesión | `first_reconciliation_conflicts_instead_of_choosing_a_winner` |
| Una edición remota entre observar y escribir no se sobrescribe | `a_remote_edit_between_observation_and_write_is_never_overwritten`, `concurrent_edit_before_write_is_preserved` |
| Una respuesta perdida se verifica observando y no se repite | `a_lost_write_response_is_verified_by_observation_and_not_repeated`, `lost_write_response_is_unconfirmed_although_applied` |
| Una observación incompleta no infiere borrados ni habilita publicar | `an_incomplete_observation_is_reported_and_never_infers_deletions`, `failed_list_call_is_an_error_not_an_empty_page` |
| Los borrados remotos exigen `--allow-deletes` y el plan se valida completo antes de escribir | `push_publishes_a_fixed_commit_with_explicit_deletions_and_verifies_it` |
| Un esquema de herramienta distinto al perfil desactiva esa operación | `verify_reports_capabilities_and_schema_drift_blocks_publication`, `changed_schema_disables_only_that_capability` |
| Los cambios de modo son explícitos y no contactan al servidor | `mode_transitions_are_explicit_and_contact_no_server` |
| La autorización verifica PKCE, `resource` y `state`; los tokens no aparecen en errores | `login_verifies_pkce_sends_resource_and_forwards_scope`, `tampered_state_requires_login_and_never_exchanges_the_code`, `refresh_adopts_a_rotated_refresh_token` |

Toda corrección de pérdida de datos requiere una regresión con documentos temporales y un control explícito de qué archivos deben sobrevivir. Las limitaciones que no tienen una prueba aparecen en [SECURITY.md](SECURITY.md) y [ARCHITECTURE.md](ARCHITECTURE.md), no se presentan como garantías.
