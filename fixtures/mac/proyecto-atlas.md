# Proyecto Atlas — recuperación del servicio

> Documento completamente sintético para pruebas del MVP. No describe sistemas reales.

## Objetivo

Restaurar el servicio ficticio `atlas-sandbox` después de una interrupción controlada sin reutilizar datos reales.

## Procedimiento

1. Declarar una ventana de mantenimiento de prueba.
2. Seleccionar el snapshot sintético más reciente cuyo estado sea `verified`.
3. Restaurar el snapshot en el entorno aislado `atlas-sandbox`.
4. Ejecutar la validación sintética v3: comprobar salud, lectura y una escritura descartable.
5. Habilitar el tráfico de prueba al 10 %, observar cinco minutos y luego ampliar al 100 %.
6. Registrar el identificador de revisión y cerrar la ventana.

## Criterio de rollback

Si dos comprobaciones consecutivas fallan, volver al snapshot anterior y mantener el tráfico deshabilitado. El responsable y la fecha objetivo se mantienen en la estación Windows para forzar una búsqueda federada.
