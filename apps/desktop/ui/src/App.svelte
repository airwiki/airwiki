<script lang="ts">
  import { BookOpen, CheckCircle2, Search, Settings2, Sparkles } from '@lucide/svelte';
  import { onMount } from 'svelte';
  import { addCollection, connect, pickCollectionFolder, rescanCollection, searchKnowledge, type AppSnapshot, type FolderSelection } from './api';

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
  let folderSelection: FolderSelection | null = null;
  let collectionName = '';
  let question = '';
  let includePublic = false;
  let actionMessage = '';
  let actionBusy = false;

  onMount(() => {
    connect((event) => {
      snapshot = event.snapshot;
      if (event.snapshot.search?.status !== 'searching') actionBusy = false;
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

  async function chooseFolder() {
    actionMessage = '';
    try {
      folderSelection = await pickCollectionFolder();
    } catch {
      actionMessage = 'No se pudo abrir el selector. Inténtalo de nuevo.';
    }
  }

  async function createCollection() {
    if (!folderSelection) return;
    actionBusy = true;
    try {
      await addCollection(collectionName, folderSelection.token);
      collectionName = '';
      folderSelection = null;
      actionMessage = 'Colección añadida. El análisis comenzó en segundo plano.';
    } catch {
      actionMessage = 'No se pudo añadir la colección. Vuelve a elegir la carpeta e inténtalo de nuevo.';
      folderSelection = null;
    } finally {
      actionBusy = false;
    }
  }

  async function submitSearch() {
    actionBusy = true;
    try {
      await searchKnowledge(question, includePublic);
      actionMessage = 'Buscando evidencia autorizada…';
    } catch {
      actionMessage = 'La búsqueda no pudo comenzar. Comprueba que AirWiki esté listo.';
      actionBusy = false;
    }
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
              <article><div><strong>{collection.name}</strong><small>{collection.documentCount} documentos · {collection.publishedCount} publicados</small></div><button class="text-action" onclick={() => rescanCollection(collection.id)}>Analizar cambios</button></article>
            {/each}
          </div>
        {/if}

        {#if destination === 'library'}
          <form class="action-panel" onsubmit={(event) => { event.preventDefault(); createCollection(); }}>
            <label><span>Nombre de la colección</span><input bind:value={collectionName} maxlength="120" placeholder="Ej. Manuales del taller" required /></label>
            <div><button type="button" class="secondary" onclick={chooseFolder}>Elegir carpeta</button><small>{folderSelection?.displayPath ?? 'AirWiki solo leerá la carpeta que elijas.'}</small></div>
            <button class="primary" disabled={actionBusy || !folderSelection || !collectionName.trim()}>Añadir a Biblioteca</button>
          </form>
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

        {#if destination === 'search'}
          <form class="search-panel" onsubmit={(event) => { event.preventDefault(); submitSearch(); }}>
            <label for="knowledge-question">Pregunta a tu conocimiento</label>
            <textarea id="knowledge-question" bind:value={question} maxlength="4096" rows="4" placeholder="¿Qué evidencia tenemos sobre…?" required></textarea>
            <label class="check"><input type="checkbox" bind:checked={includePublic} /> Incluir colecciones públicas disponibles</label>
            <button class="primary" disabled={actionBusy}>Buscar evidencia</button>
          </form>
          {#if snapshot?.search}
            <div class="search-results" aria-live="polite">
              <p class="section-label">{snapshot.search.status === 'searching' ? 'Resultados parciales' : 'Evidencia encontrada'}</p>
              {#each snapshot.search.hits as hit}
                <article><small>{hit.headingOrPage}</small><h3>{hit.title}</h3><p>{hit.snippet}</p><code>{hit.logicalResourceUri}</code></article>
              {:else}
                {#if snapshot.search.status === 'complete'}
                  <p class="empty">No encontramos evidencia que responda claramente. Prueba una pregunta más específica.</p>
                {/if}
              {/each}
            </div>
          {/if}
        {/if}
        {#if actionMessage}<p class="action-message" aria-live="polite">{actionMessage}</p>{/if}
      </div>
    </section>
  </main>
</div>
