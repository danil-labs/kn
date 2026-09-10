# Perfiles de herramientas MCP

Un perfil es la revisión humana de un servidor MCP concreto. Dice qué herramienta usa kn para cada operación documental, cómo arma sus argumentos y dónde lee cada dato del resultado. MCP define cómo descubrir e invocar herramientas; no define operaciones de carpetas. kn nunca deduce una acción de la descripción de una herramienta.

`kn remote add` guarda una copia privada del perfil bajo KN_HOME; editar el archivo original después no cambia el remoto. El ejemplo completo y probado es [profiles/reference.json](profiles/reference.json), el perfil del servidor de referencia de las pruebas. No representa a ningún proveedor.

## Estructura

| Campo | Significado |
| --- | --- |
| `profile_version` | `1` |
| `name`, `description` | Identificación del perfil |
| `result_source` | `structured` usa `structuredContent`; `text_json` interpreta como JSON el texto del resultado |
| `content_encoding` | `base64`, o `utf8` si el servidor solo transmite texto (los documentos binarios quedan fuera) |
| `operations` | Operación de kn → herramienta del servidor |
| `kinds` | Valores del campo de tipo que significan `file`, `folder` o `remote_only` |
| `errors` | JSON pointer al código de error de un resultado `isError`, y los valores que significan `precondition_failed`, `already_exists` y `not_found` |

Cada operación tiene estos campos:

- `tool`: nombre exacto de la herramienta.
- `schema_sha256`: fijación del esquema.
- `arguments`: plantilla JSON de los argumentos.
- `result`: campo → JSON pointer dentro del resultado.
- `fails_if_exists`: solo en las operaciones de creación.

## Operaciones

| Operación | Marcadores obligatorios | Campos de resultado | Uso |
| --- | --- | --- | --- |
| `identify` | `{folder_id}` | `id`, `kind` (`name` opcional) | `kn remote verify`: la raíz existe y es carpeta |
| `list` | `{folder_id}`; `{cursor}` si hay `next_cursor` | `items`, `item_id`, `item_name`, `item_kind`; `item_revision` para archivos; `next_cursor` opcional | Enumerar una carpeta. Los punteros `item_*` son relativos a cada elemento |
| `read` | `{file_id}` | `content`, `revision` | Bytes y la revisión ligada a esos bytes |
| `update` | `{file_id}`, `{content}`, `{expected_revision}` | `id`, `revision` opcionales | Reemplazar contenido si la revisión coincide |
| `create` | `{folder_id}`, `{name}`, `{content}` y `fails_if_exists: true` | `id`, `revision` opcionales | Crear sin sobrescribir |
| `create_folder` | `{folder_id}`, `{name}` y `fails_if_exists: true` | `id` | Crear una carpeta intermedia |
| `delete` | `{file_id}`, `{expected_revision}` | — | Borrar si la revisión coincide |

`list` y `read` son obligatorias. Las demás son opcionales: un perfil sin escrituras sirve para observar en `desktop_sync_observed`, y kn bloquea completo cualquier plan que necesite una operación ausente.

## Marcadores

Solo se sustituye un string que sea exactamente `{marcador}`. Los marcadores válidos son:

- `folder_id`
- `cursor`
- `file_id`
- `name`
- `content`
- `expected_revision`

Un marcador opcional sin valor elimina su clave; por ejemplo, el cursor de la primera página. Cualquier otro texto se envía literal. Un marcador desconocido invalida el perfil.

## Fijación de esquema

`schema_sha256` es el SHA-256 del JSON canónico (claves ordenadas, sin espacios) de `{"inputSchema": …, "outputSchema": …}`, tal como la herramienta aparece en `tools/list`. Un esquema ausente cuenta como `null`. El nombre y la descripción no participan.

`kn remote verify <alias>` muestra, por operación, el hash esperado y el real. Si difieren, la operación queda no disponible (`PROFILE_MISMATCH`) hasta que alguien revise el esquema nuevo, actualice el perfil y lo pruebe.

## Garantías que declara el perfil

- `update` y `delete` sin `{expected_revision}` no están disponibles: kn no escribe sin una condición de revisión que aplique el servidor.
- `create` y `create_folder` sin `fails_if_exists` no están disponibles. Esa declaración es la que evita duplicados después de una respuesta perdida.
- Son declaraciones: se demuestran con documentos sintéticos, dos cuentas y fallos inyectados antes de certificar un servidor. Una declaración falsa invalida las garantías de kn.

Los resultados `isError` se clasifican con `errors.pointer`; un código desconocido es un rechazo sin clasificar. Tratan como resultado desconocido, verificado por la siguiente observación:

- los errores de transporte;
- las respuestas 5xx;
- los flujos que terminan sin respuesta;
- los errores JSON-RPC que no son de protocolo.

## Revisar un servidor nuevo

1. Lee `tools/list` del servidor y elige, para cada operación, la herramienta cuyo contrato cumpla la tabla anterior.
2. Escribe el perfil, ejecuta `kn remote add` y `kn remote verify`, revisa los esquemas reales y copia sus hashes.
3. Prueba con documentos sintéticos:
   - paginación;
   - nombres con acentos y espacios;
   - binarios;
   - conflictos;
   - edición entre consulta y escritura;
   - respuesta perdida;
   - dos cuentas.
4. Registra los resultados por destino en [VALIDATION.md](VALIDATION.md) antes de declarar el proveedor.
