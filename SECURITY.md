# Seguridad y conservación de documentos

kn es experimental. Guarda y restaura documentos mediante Git, pero no es un sandbox para agentes ni un sistema de respaldo completo. Incluye un cliente MCP con autorización OAuth (PKCE) y credenciales en el almacén del sistema. No incluye adaptadores directos de proveedores ni proveedores certificados.

## Alcance

- KN_HOME y Git en PATH forman parte del entorno confiable. No compartas un motor escribible con usuarios o procesos no confiables.
- Los locks solo coordinan operaciones kn en esta máquina. No bloquean editores, clientes cloud ni Git ejecutado directamente.
- Un checkpoint puede capturar archivos en momentos distintos si otra aplicación escribe durante la operación. No hay garantía transaccional de carpeta ni recuperación automática después de un fallo de disco o cierre abrupto.
- Los archivos ignorados y carpetas vacías no tienen respaldo en el historial. Un archivo que pasa a estar versionado deja de ser ignorado según la semántica Git.
- Los objetos Git y gitfiles no se suben a carpetas sincronizadas. No hay detección universal de directorios cloud ni prueba de que un proveedor esté sincronizado.
- Los commits locales atribuyen la observación a kn; no prueban identidad de personas o dispositivos.
- Los archivos y metadatos malformados deben producir un error, no disparar comandos desde su contenido. No hay garantía contra cambios de symlink entre la comprobación y el uso.
- Los tokens de servidores MCP viven en el almacén seguro del sistema o llegan en `KN_MCP_ACCESS_TOKEN`. Nunca van en KN_HOME, documentos, argumentos, diarios ni mensajes de error. En Linux se usa el keyring del kernel, que no persiste tras reiniciar.
- kn solo envía el token al endpoint configurado (https, o http en loopback) y no sigue redirecciones. La metadata de autorización debe declarar el mismo recurso y soportar S256.
- El operador del servidor MCP recibe acceso a los documentos: autorizarlo es decisión de la persona. El perfil es un dato que kn copia al configurarlo; nada de una carpeta compartida se ejecuta.
- Las escrituras remotas exigen precondición de revisión o creación exclusiva según el perfil. Un perfil que declara garantías que el servidor no cumple anula esa protección; por eso cada servidor se prueba antes de certificarlo.

## Reportar un problema

Comunica al administrador del repositorio por un canal privado existente un caso mínimo reproducible con documentos sintéticos. Si GitHub muestra «Report a vulnerability», puede usarse ese canal privado. No adjuntes documentos de clientes, tokens, rutas sensibles ni el contenido completo de KN_HOME en issues o PRs. No se publica un SLA ni una auditoría de seguridad independiente.

Para un fallo que pudo causar pérdida de datos: detén operaciones sobre esa carpeta, conserva principal, sesiones y KN_HOME antes de intentar recuperar. El historial y los cambios no guardados son piezas diferentes del respaldo. [Arquitectura](ARCHITECTURE.md) describe dónde vive cada una.
