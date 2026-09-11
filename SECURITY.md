# Seguridad y conservación de documentos

kn es experimental. Guarda y restaura documentos mediante Git, pero no es un sandbox para agentes ni un sistema de respaldo completo. La versión actual no tiene OAuth, almacenamiento de tokens ni adaptadores cloud.

## Alcance

- KN_HOME y Git en PATH forman parte del entorno confiable. No compartas un motor escribible con usuarios o procesos no confiables.
- Los locks solo coordinan operaciones kn en esta máquina. No bloquean editores, clientes cloud ni Git ejecutado directamente.
- Un checkpoint puede capturar archivos en momentos distintos si otra aplicación escribe durante la operación. No hay garantía transaccional de carpeta ni recuperación automática después de un fallo de disco o cierre abrupto.
- Los archivos ignorados y carpetas vacías no tienen respaldo en el historial. Un archivo que pasa a estar versionado deja de ser ignorado según la semántica Git.
- Los objetos Git y gitfiles no se suben a carpetas sincronizadas. No hay detección universal de directorios cloud ni prueba de que un proveedor esté sincronizado. kn solo reconoce, por metadatos de macOS y Windows, documentos que el proveedor todavía no descargó; esos quedan fuera del historial hasta descargarse.
- Los commits locales atribuyen la observación a kn; no prueban identidad de personas o dispositivos.
- Los archivos y metadatos malformados deben producir un error, no disparar comandos desde su contenido. No hay garantía contra cambios de symlink entre la comprobación y el uso.

## Reportar un problema

Comunica al administrador del repositorio por un canal privado existente un caso mínimo reproducible con documentos sintéticos. Si GitHub muestra «Report a vulnerability», puede usarse ese canal privado. No adjuntes documentos de clientes, tokens, rutas sensibles ni el contenido completo de KN_HOME en issues o PRs. No se publica un SLA ni una auditoría de seguridad independiente.

Para un fallo que pudo causar pérdida de datos: detén operaciones sobre esa carpeta, conserva principal, sesiones y KN_HOME antes de intentar recuperar. El historial y los cambios no guardados son piezas diferentes del respaldo. [Arquitectura](ARCHITECTURE.md) describe dónde vive cada una.
