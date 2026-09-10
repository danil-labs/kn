# Contribuir

Lee [AGENTS.md](AGENTS.md) para comandos, mapa y reglas de seguridad, y [ARCHITECTURE.md](ARCHITECTURE.md) para entender los límites del núcleo. Las instrucciones aplican por igual a personas y agentes.

## Flujo

1. Describe el problema y el cambio observable. Para cambios de contrato, muestra ejemplos antes/después.
2. Trabaja en una rama y conserva Cargo.lock. No pruebes operaciones destructivas sobre documentos reales.
3. Ejecuta los cuatro checks de AGENTS.md. Las regresiones deben comprobar archivos preservados, salida o comportamiento, no copiar la implementación.
4. Actualiza la documentación que corresponda y CHANGELOG.md. No declares capacidades futuras como disponibles.
5. Abre un PR con alcance, comprobaciones y pendientes. Espera los checks Linux/macOS/Windows antes de integrar; no uses un bypass administrativo para esconder fallos.

La CI se define en [.github/workflows/ci.yml](.github/workflows/ci.yml). Un consumidor debe probar su integración al modificar el protocolo. La existencia de una salida de kn no demuestra que Terminus ya la consuma.

## Publicación

El repositorio comienza con una versión local experimental, 0.3.0 en Cargo.toml, sin release etiquetado ni instalación automática desde Terminus. `cargo build --workspace --locked` permite construir desde el checkout. No publicar en crates.io, cambiar visibilidad ni elegir una licencia por inferencia: esas son decisiones del propietario. Los manifiestos deshabilitan publicación en crates.io mientras se define la distribución.

La futura distribución necesita artefactos por plataforma, checksums y pruebas del instalador. La lista abierta del issue #2 no debe cerrarse por entregar únicamente el núcleo local.
