<script lang="ts">
  import { BookOpen, CheckCircle2, Search, Settings2, Sparkles } from '@lucide/svelte';
  import { onMount } from 'svelte';
  import { connect, type AppSnapshot } from './api';

  type Destination = 'library' | 'review' | 'search' | 'system';

  const destinations = [
    { id: 'library', label: 'Biblioteca', icon: BookOpen },
    { id: 'review', label: 'Revisión', icon: CheckCircle2 },
    { id: 'search', label: 'Buscar', icon: Search },
    { id: 'system', label: 'Sistema', icon: Settings2 }
  ] as const;

  let destination: Destination = 'library';
  let runtimeLabel = 'Preparando servicios privados';
  let snapshot: AppSnapshot | null = null;

  onMount(() => {
    connect((event) => {
      snapshot = event.snapshot;
      runtimeLabel = event.snapshot.phase === 'ready' ? 'Servicios privados listos' : 'Preparando servicios privados';
    }).then((initial) => {
      snapshot = initial;
      runtimeLabel = initial.phase === 'ready' ? 'Servicios privados listos' : runtimeLabel;
    }).catch(() => { runtimeLabel = 'Vista previa sin runtime nativo'; });
  });

  function select(next: Destination) {
    destination = next;
    window.location.hash = next;
  }
</script>

<svelte:head><meta name="theme-color" content="#07131f" /></svelte:head>

<div class="shell">
  <aside class="rail" aria-label="Navegación principal">
    <div class="brand"><span class="brand-mark">A</span><span>AirWiki</span></div>
    <nav>
      {#each destinations as item}
        <button class:active={destination === item.id} onclick={() => select(item.id)}>
          <item.icon size={18} strokeWidth={1.8} aria-hidden="true" />
          <span>{item.label}</span>
        </button>
      {/each}
    </nav>
    <div class="device-state">
      <span class="pulse" aria-hidden="true"></span>
      <div><strong>En este dispositivo</strong><small>{runtimeLabel}</small></div>
    </div>
  </aside>

  <main>
    <header>
      <div><p class="eyebrow">Espacio de conocimiento local</p><h1>Tu evidencia, lista para comprobar.</h1></div>
      <button class="primary"><Sparkles size={17} />Siguiente acción</button>
    </header>

    <section class="workspace" aria-live="polite">
      <div class="evidence-rail" aria-hidden="true"><i></i><i></i><i></i></div>
      <div class="content">
        <p class="section-label">{destinations.find((item) => item.id === destination)?.label}</p>
        <h2>{destination === 'library' ? 'De archivos locales a conocimiento verificable' : destinations.find((item) => item.id === destination)?.label}</h2>
        <p class="lede">AirWiki mantiene cada afirmación unida a su fuente, su revisión humana y sus permisos de publicación.</p>

        <div class="sequence">
          <article><span>Fuente</span><strong>{snapshot?.collections.length ?? 0} carpetas elegidas</strong><p>Los originales permanecen donde están.</p></article>
          <article><span>Preparación</span><strong>{snapshot?.collections.reduce((total, item) => total + item.documentCount, 0) ?? 0} documentos locales</strong><p>La IA propone; nunca publica ni concede acceso.</p></article>
          <article><span>Decisión</span><strong>{snapshot?.reviews.length ?? 0} revisiones pendientes</strong><p>Comprueba la evidencia antes de hacerla visible.</p></article>
        </div>

        {#if destination === 'library' && snapshot?.collections.length}
          <div class="records" aria-label="Colecciones">
            {#each snapshot.collections as collection}
              <article><div><strong>{collection.name}</strong><small>{collection.documentCount} documentos · {collection.publishedCount} publicados</small></div><span>{collection.needsReviewCount} por revisar</span></article>
            {/each}
          </div>
        {/if}

        {#if destination === 'review' && snapshot}
          <div class="records" aria-label="Revisiones pendientes">
            {#each snapshot.reviews as review}
              <article><div><strong>{review.sourceName}</strong><small>{review.collectionName} · revisión {review.sourceRevision}</small></div><span>Ver evidencia</span></article>
            {:else}<p class="empty">No hay propuestas pendientes. Los próximos cambios aparecerán aquí.</p>{/each}
          </div>
        {/if}

        {#if destination === 'system' && snapshot}
          <div class="records"><article><div><strong>{snapshot.model?.displayName ?? 'IA local'}</strong><small>{snapshot.model?.active ? 'Modelo activo' : 'Requiere preparación'}</small></div><span>{snapshot.peers.length} equipos</span></article></div>
        {/if}
      </div>
    </section>
  </main>
</div>
