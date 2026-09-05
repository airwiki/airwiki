# Plan de rediseño de la interfaz de AirWiki

Estado: Activo; implementación y validación autorizadas.
Fecha: 2026-09-05.

## Resultado y alcance

Convertir AirWiki en un espacio de lectura, búsqueda y revisión de conocimiento:
una navegación lateral discreta, contenido protagonista y fuentes consultables
en contexto. La interfaz debe funcionar en macOS y Windows, desde 1024×720,
conservar la autonomía local y explicar las decisiones de acceso sin llenar
cada pantalla de controles técnicos.

La aceptación del diseño incluye el símbolo de **W enlazada** para las wikis.
El [vector de referencia](assets/wiki-linked-w.svg) forma parte de este plan;
el componente `WikiIcon` utiliza esta geometría. El logo oficial de
AirWiki conserva su identidad. Las alternativas de iniciales, ausencia de
iconos y controles de experimentación de la maqueta no forman parte de la
primera implementación.

Este documento describe el estado objetivo. Las
[guías de diseño vigentes](ui-design-guidelines.md) y las reglas de producto
siguen siendo la autoridad sobre el comportamiento actual. El plan se enlaza
desde [PLANS.md](../PLANS.md) como activo. El trabajo de instaladores queda en
pausa, conservando sus pendientes y sin crear una dependencia con su firma pública.

## Camino mínimo de aceptación

1. Abrir una wiki local con 50 conceptos en una ventana de 1024×720 y leer el
   concepto seleccionado sin recorrer antes toda la lista.
2. Encontrar otro concepto desde el índice o la búsqueda; abrirlo y volver
   conservando el contexto de navegación.
3. Consultar su estado y sus fuentes. Distinguir una fuente declarada del
   concepto de una cita que respalda un fragmento concreto.
4. Revisar un borrador con evidencia vigente, editar su propuesta, aprobarlo y
   continuar. Probar también evidencia obsoleta, fallo, reintento y salida con
   cambios sin guardar.
5. Inspeccionar por separado Compartir y Apps de IA. La navegación y la
   aprobación no amplían permisos ni activan búsquedas públicas.
6. Cerrar y abrir de nuevo: restaurar una página local válida o volver a
   Biblioteca de forma segura si ya no está disponible.

## Decisiones de diseño

- **Estructura:** Biblioteca y Por revisar son destinos globales; la selección
  de wiki y su índice de conceptos comparten una columna lateral contextual.
  Ajustes queda al pie. La navegación normal usa dos paneles; Fuentes o
  Detalles puede añadir un tercero cuando haya espacio.
- **Lector:** título, estado esencial y contenido primero. El índice conserva
  acceso a conceptos, portada, historial y estados borrador/revisado/excluido.
  Grafo y detalles permanecen disponibles como vistas secundarias.
- **Ventanas:** panel lateral plegable y redimensionable, inicialmente de unos
  224 px, acotado por las necesidades del contenido. En 1024 px, Fuentes se
  abre de forma temporal sin comprimir el artículo ni crear un cuarto panel.
  La columna de lectura tiene un máximo aproximado de 65–72 caracteres.
- **Color y tipo:** superficies opacas neutras; azul para acción, selección y
  foco. Space Grotesk para títulos, Atkinson para lectura y tipografía del
  sistema para controles. UI de 13 px en macOS y 14 px en Windows, lectura de
  16–17 px, metadatos legibles de 11–12 px. Los estados conservan sus colores
  semánticos y siempre incluyen texto o iconografía.
- **Acceso:** dos controles identificables, Compartir y Apps de IA, con resumen
  breve y paneles propios. Los errores de publicación, permisos y vigencia
  permanecen visibles; no se relegan a un inspector cerrado.
- **Revisión:** espacio dedicado con cola y avance, conservando la comparación
  entre evidencia y propuesta. No se incorpora aprobación masiva.
- **Continuidad:** reanudar la última página local tras cargar y validar el
  estado real. La primera ejecución y la ausencia de una selección válida
  desembocan en Biblioteca. Una selección remota no provoca consultas de red
  automáticas al reiniciar.

## Dependencias confirmadas en el código

| Punto | Situación actual | Consecuencia para el plan |
| --- | --- | --- |
| Ventana mínima | La regla de 1040 px apila `.file-list` antes de `.file-preview`; la lista no limita su altura | Corregir primero el recorrido de lectura con una lista larga |
| Estilos | `styles.css` acumula capas de rediseño y sobrescrituras | Consolidar los selectores al migrar cada superficie, sin una reescritura global previa |
| Fuentes | `KnowledgeConceptSummary.sources` contiene metadatos; `KnowledgeBlock` no contiene referencias de fuente por fragmento | El inspector inicial puede mostrar fuentes declaradas; las citas precisas necesitan trazabilidad explícita |
| Evidencia | La revisión recibe extractos acotados a concepto y revisión, con estados ready/stale/missing/failed | Mantener esas comprobaciones al mover la revisión fuera del diálogo |
| Navegación | La selección vive en `App.svelte`; las preferencias no guardan la última página | La restauración entre sesiones requiere persistencia y un contrato tipado |
| Búsqueda | Ya existe búsqueda automática, consentimiento público por consulta y retorno al contexto anterior | Mantener esos comportamientos; no copiar las limitaciones de la maqueta |
| Pruebas | Hay E2E que fijan el scroll actual y baselines principalmente de Biblioteca/Ajustes | Actualizar expectativas por comportamiento y ampliar la matriz al lector y la revisión |

Fuentes de implementación:
[App.svelte](../apps/desktop/ui/src/App.svelte),
[styles.css](../apps/desktop/ui/src/styles.css),
[contrato generado](../apps/desktop/ui/src/generated/ui-contract.ts),
[preferencias](../apps/desktop/src/model_config.rs) y
[E2E](../apps/desktop/ui/e2e/onboarding.spec.ts).

## Entregas

Las fases se ejecutan en orden. Cada PR debe dejar un recorrido utilizable;
una fase puede dividirse en varios PR si mezcla presentación y persistencia.
No se presupone una fecha de entrega antes de validar las dependencias.

### 1. Corregir la ventana mínima y establecer la base visual

- **PR 1a:** impedir que el índice largo desplace el lector en 1024×720. Usar
  un selector o panel plegable cuando las dos columnas no sean utilizables;
  mantener la selección y devolver el foco al cerrarlo.
- **PR 1b:** incorporar un componente `WikiIcon` con la W enlazada, compartido
  por biblioteca y navegación. Ajustar ópticamente 16, 20 y 24 px, selección,
  claro, oscuro y alto contraste; no usar el icono como indicador de permiso.
- Centralizar los roles de superficie, texto, acción, foco y espaciado usados
  por estas superficies. Migrar gradualmente las reglas antiguas y quitar
  únicamente las que hayan dejado de tener consumidores.
- Adelantar la jerarquía básica del lector: estado esencial junto al título y
  ficha ampliada después del contenido. La prueba con 50 conceptos confirmó
  que limitar el índice no basta si la ficha sigue ocultando el primer párrafo.

**Aceptación:** wiki con 50 conceptos y títulos largos; el título y el comienzo
del artículo seleccionado son visibles sin desplazar la lista. El índice es
operable por teclado. El símbolo se reconoce en todos sus tamaños.

### 2. Navegación y biblioteca

- Construir el shell lateral y una barra de búsqueda compacta. La wiki activa
  despliega su índice en la misma columna; el artículo usa el área principal.
- Biblioteca presenta filas con nombre, descripción breve, conceptos,
  pendientes y resumen de compartición. Mantener orden y filtros útiles;
  reducir estados repetidos y explicaciones de uso permanentes.
- Conservar el catálogo público como destino explícito y los orígenes local,
  cercano y público. Mantener creación desde carpeta, importación y memoria,
  con un estado vacío que ofrezca una siguiente acción concreta.
- Extraer componentes de presentación de `App.svelte` a medida que se migra
  cada vista. Las solicitudes, snapshots y reglas de autorización siguen en
  sus límites actuales; no introducir otro router o gestor de estado.
- Mantener atrás/adelante, atajos contextuales, foco y selección. Añadir modo
  de lectura con la barra lateral oculta y una forma visible de recuperarla.

**Aceptación:** crear/abrir/cambiar una wiki y sus conceptos con ratón y teclado;
volver desde Ajustes y desde un resultado sin perder el contexto de esa sesión.
Abrir Biblioteca no inicia exploración pública.

### 3. Lectura, fuentes y detalles

- Eliminar la identidad duplicada y la ficha técnica situada antes del texto.
  Mantener el estado esencial cerca del título; mover metadatos ampliados al
  inspector y las fuentes al margen o a una superficie temporal.
- Reutilizar la misma jerarquía visual en el visor de wikis remotas, conservando
  sus restricciones de solo lectura. Mantener historial, grafo, enlaces
  relacionados, contenido truncado y recuperación de páginas no disponibles.
- **3a, con el contrato actual:** mostrar título, recurso, autor y fecha de las
  fuentes cuando existan. No prometer extractos ni navegación a documentos
  originales que ese origen no pueda proporcionar.
- **3b, citas precisas:** inspeccionar el marcado y los localizadores existentes.
  Si hay una correspondencia determinista, transportarla con una referencia
  tipada y ligada a la revisión. Si falta, mostrar "Fuentes del concepto" sin
  asignar números a párrafos. La IA y el frontend no inventan asociaciones.
- Una ampliación necesaria del contrato se hace en Rust y se regenera el
  TypeScript. No se modifica el formato OKF, la publicación ni la exposición de
  documentos fuente remotos para reproducir el aspecto de la maqueta.

**Aceptación:** artículo corto y largo; cero, una y varias fuentes; metadatos
incompletos; referencia rota; revisión cambiada. Las citas presentes conducen
exactamente a la evidencia de su revisión. El caso sin trazabilidad permanece
explícito y legible. Fallos o conocimiento retirado nunca parecen vigentes.

### 4. Resultados de búsqueda

- Priorizar título de concepto, fragmento y procedencia. Reducir la cabecera
  del grupo de wiki a una línea y llevar compatibilidad/metadatos ampliados a
  detalles. Conservar agrupación por origen, propietario y wiki, orden del
  backend, límite de coincidencias y total real de resultados.
- Mantener búsqueda tras pausa al escribir, Enter inmediato, composición de
  texto y protección contra respuestas antiguas. Los filtros actúan sobre
  resultados existentes sin repetir consultas.
- El alcance local/cercano/público permanece visible. Editar una consulta no
  hereda silenciosamente su consentimiento público anterior.
- Conservar resultados seguros durante carga parcial y ofrecer estados
  diferentes para sin resultados, origen desconectado y fallo con reintento.

**Aceptación:** abrir el concepto exacto y volver a consulta, filtro y posición;
resultados locales/cercanos/públicos sintéticos; respuesta antigua ignorada;
cobertura incompleta visible; ninguna nueva salida de consultas a la red.

### 5. Revisión como recorrido dedicado

- Crear Por revisar con cola global y agrupación por wiki. Distinguir pendientes
  accionables, elementos temporalmente bloqueados y excluidos recuperables.
- Mostrar fuente y propuesta en paralelo donde quepan; en espacio reducido,
  permitir alternarlas conservando el borrador editado. Acciones accesibles
  durante la comparación y un indicador de progreso basado en datos reales.
- Conservar Aprobar y continuar, Más tarde y Excluir. Salir con cambios editados
  pide una decisión concreta sobre esos cambios; navegar no los descarta.
- Avanzar después del resultado confirmado, conservando la posición ante
  cambios de la cola. Un fallo permite reintentar sin duplicar decisiones.
- Mantener concepto, revisión, request ID, evidencia vigente, restricciones de
  solo lectura y reanálisis. Aprobar no equivale a verificar o compartir; sí
  puede hacer que el conocimiento sea accesible bajo permisos ya existentes,
  y el texto de la acción no debe sugerir lo contrario.

**Aceptación:** aprobar, posponer, excluir y revisar un excluido; cambio de
fuente durante la revisión; evidencia ausente/fallida; doble activación;
reanálisis en curso; salida con edición; finalización de la cola y foco correcto.

### 6. Ajustes y restauración de sesión

- Simplificar General/IA local, Conexiones y Apps de IA sin ocultar instalación,
  licencia, tamaño de descarga, estado, error o siguiente acción necesaria.
  Acortar la explicación introductoria cuando la configuración esté lista.
- Sustituir el indicador circular difícil de interpretar por un acceso a
  Ajustes con estado breve cuando haga falta. Reutilizar la derivación de
  `systemStatus.ts`; la salud de una wiki sigue perteneciendo a esa wiki.
- Mantener paneles separados de Compartir y Apps de IA, permisos por wiki,
  consentimiento público por app y confirmaciones nativas existentes.
- En un PR separado, persistir la selección local y la geometría de paneles
  mediante el worker y el almacenamiento operativo SQLite existente. Guardar
  identificadores y valores acotados, nunca consultas, extractos, contenido o
  rutas de documentos originales. No crear otro almacén en `localStorage`.
- Restaurar solo después del snapshot inicial y de revalidar la selección.
  Para wiki eliminada, concepto ausente, estado inválido o último destino
  remoto, volver a Biblioteca con una salida comprensible. Las rutas explícitas
  y el onboarding tienen precedencia; no restaurar operaciones pendientes.
- Los campos persistidos y cualquier migración requieren compatibilidad,
  límites y una estrategia de recuperación. El guardado se agrupa y sale del
  hilo de UI; un error al guardar no impide seguir leyendo.

**Aceptación:** iniciar/cancelar/reintentar preparación; permisos revocados;
ajustes sin guardar; cierre y reapertura; selección eliminada; escritura fallida;
configuración antigua y panel fuera de los límites de una ventana más pequeña.

### 7. Coherencia final y aceptación instalada

- Revisar todas las superficies migradas en ambos idiomas y plataformas;
  retirar selectores y componentes obsoletos tras comprobar sus consumidores.
- Consolidar el mapa de tokens definitivo, actualizar documentación de diseño,
  búsqueda, instalación, capturas sintéticas y CHANGELOG según lo entregado.
- Mantener la prueba de rendimiento de navegación existente. Medir listas
  grandes antes de introducir virtualización o nuevas dependencias.
- Aplicar el proceso de [CODE_REVIEW.md](../CODE_REVIEW.md): ramas y PR enfocados,
  revisión apropiada, checks verdes y merge. Los cambios que afecten contratos,
  persistencia, revisión o acceso reciben las pruebas negativas y revisión de
  seguridad proporcionales a su efecto.

**Aceptación:** el camino completo funciona en candidatos instalados de macOS
y Windows. Una plataforma no validada se registra como pendiente, no como PASS.
La firma pública, notarización y promoción del actualizador no bloquean esta QA.

## Validación

Cada entrega cubre éxito y fallo/recuperación en los componentes afectados.
No se regeneran baselines automáticamente para hacer pasar una diferencia:
primero se revisa si el nuevo resultado cumple el diseño.

| Dimensión | Casos mínimos |
| --- | --- |
| Ventana | 1024×720, 1180×760, 1440×900; redimensionado continuo |
| Apariencia | Claro, oscuro, sistema, alto contraste de Windows |
| Accesibilidad | Teclado completo, foco visible/restaurado, VoiceOver/Narrator en recorridos principales, movimiento reducido |
| Escala e idioma | ES/EN; texto y pantalla a 125%, 150% y 200% donde el host lo permita |
| Datos sintéticos | Biblioteca vacía y poblada; 50 conceptos con títulos largos; artículo largo; varios orígenes |
| Estados | Carga, vacío, sin resultados, parcial, desconexión, error/reintento, revocación y revisión obsoleta |

La suite actual que exige `overflow: visible` debe reformularse cuando cambie
el layout: probar que se alcanza el artículo, que no hay clipping y que foco y
selección se conservan. No fijar una estructura de scroll solo por coincidir
con la implementación anterior. Las regiones laterales independientes no deben
crear scroll anidado dentro de la columna de lectura.

Para UI: `pnpm --dir apps/desktop/ui run check`, `lint`, `test`, `check:e2e` y
`build`; ejecutar los E2E relevantes con el runner existente y candidatos de
cada plataforma. Usar las versiones fijadas por el repositorio.

Para cambios Rust, aplicar el nivel correspondiente de
[AGENTS.md](../AGENTS.md). Si cambia el IPC, usar
`cargo run --locked -p xtask -- ui-bindings generate` y `ui-bindings check`;
no editar el contrato generado a mano. Las migraciones, OKF o protocolos requieren
su documentación y pruebas de compatibilidad. Dependencias nuevas requieren
los checks de licencias y `cargo deny` aplicables.

Este plan, por sí solo, se valida con
`cargo run --locked -p xtask -- docs check` y `git diff --check`.

## Límites, recuperación y cierre

- El alcance es el producto de escritorio existente en Tauri/Svelte. No incluye
  un framework nuevo, chat como navegación principal, colaboración nueva,
  personalización de iconos por wiki, telemetría o nuevos servicios externos.
- La maqueta es una referencia visual; sus datos, acciones simuladas y controles
  de diseño no se copian al producto. No establece garantías sobre evidencia.
- Las reglas vigentes de [búsqueda](search-and-federation.md),
  [amenazas](threat-model.md), [autoridad de datos](architecture.md) y
  [límites de la aplicación](../apps/AGENTS.md) se mantienen.
- Los PR visuales se pueden revertir sin migrar el conocimiento. Una ampliación
  persistida conserva migraciones append-only y lecturas compatibles; no se
  revierte borrando datos. No se mantienen dos shells completos indefinidamente.
- PLANS.md mantiene este rediseño como único plan activo. El trabajo de
  instaladores queda en pausa; sus pendientes no se consideran terminados.
- El cierre exige todas las entregas aceptadas, documentación actualizada,
  revisión y checks verdes, evidencia instalada de ambas plataformas y ninguna
  regresión de permisos. Los aplazamientos deben ser explícitos; una entrega
  visual parcial no se presenta como el rediseño completo.

## Seguimiento

La implementación de la fase 1 está preparada: W compartida, tokens
centralizados, índice acotado y contenido antes de la ficha ampliada. Pasan las
233 pruebas de UI y el recorrido E2E con IPC real en macOS, en claro y oscuro;
el caso de 50 conceptos comprueba lectura, selección y foco visible a 1024,
1180 y 1440 px. La aceptación de fase sigue abierta: faltan la revisión
independiente, la matriz de baselines y las comprobaciones instaladas, incluido
teclado real y Windows. El runner emite eventos de teclado sintéticos que no
ejecutan la activación nativa de un botón, por lo que no certifican ese recorrido.

La base de la fase 2 incorpora un marco compartido para lectura local y remota,
barra lateral plegable y redimensionable, selector de wiki e índice contextual,
filas de biblioteca simplificadas y acceso a Ajustes con avisos textuales. Por
revisar abre una cola real agrupada por wiki, todavía con el editor de evidencia
existente. Al plegar se conserva el índice y su posición; Enter en el separador
restaura el foco en el control de lectura. Un modelo que termina de prepararse
no reanuda búsquedas pendientes mientras el usuario está revisando.

La base de navegación pasa 243 pruebas en 18 archivos, `check`, `lint` y
`check:e2e`. El recorrido de IPC real en macOS cubre la biblioteca, el índice de
50 conceptos, artículo corto y largo, plegado sin perder el desplazamiento y
ajuste con flechas. Se inspeccionaron capturas del lector en claro y oscuro y
se corrigió la alineación de descripciones en la biblioteca. La comparación de
baselines permanece desactivada hasta consolidar las superficies del rediseño.

El regreso explícito desde Ajustes conserva el scroll de lectura e índice;
el visor remoto mantiene la página y el modo de vista del mismo browse vigente.
Al descartar preferencias editadas se continúa el destino o acción solicitada,
incluidos Por revisar, Nueva wiki y el atajo de búsqueda. Cancelar conserva las
ediciones. La disponibilidad remota actual sigue prevaleciendo sobre la selección.

El historial local distingue wikis y páginas dentro de la sesión y restaura
selección, vista, filtro y desplazamiento tras obtener el bundle y la página
vigentes. El navegador guarda únicamente identificadores opacos; las
coordenadas se conservan en memoria acotada. La restauración usa request IDs
desde antes del envío y no reutiliza fingerprints antiguos. Una wiki ausente
o entrada caducada vuelve a Biblioteca; una página ausente conserva el índice
usable y un fallo permite reintentar. El selector conserva el índice al abrirse.
El recorrido con IPC real en macOS también comprueba Atrás/Adelante entre
artículos y sus posiciones de lectura. Esta entrega pasa 251 pruebas de UI;
incluyen revisión cambiada, respuesta fuera de orden, fallo con reintento,
selección eliminada y salida de Ajustes con edición pendiente.

Siguen pendientes la reducción de la cabecera de lectura en fase 3, el contexto
completo de resultados de búsqueda, el editor dedicado y la persistencia local.
La base visual y sus pruebas no completan por sí solas la aceptación instalada.

- [ ] Fase 1: ventana mínima, W enlazada y base visual.
- [ ] Fase 2: navegación y biblioteca.
- [ ] Fase 3: lector, fuentes y detalles.
- [ ] Fase 4: búsqueda.
- [ ] Fase 5: revisión dedicada.
- [ ] Fase 6: ajustes y restauración local.
- [ ] Fase 7: coherencia final y aceptación instalada.
