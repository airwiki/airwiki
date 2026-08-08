<script lang="ts">
  import { BookOpen, CheckCircle2, FileText, History, RefreshCw, Search, Settings2, Sparkles } from '@lucide/svelte';
  import { listen } from '@tauri-apps/api/event';
  import { onMount } from 'svelte';
  import { addCollection, approveReview, cancelModelInstall, connect, hideToTray, installModels, loadKnowledgeBundle, loadKnowledgePage, loadReviewEvidence, pickCollectionFolder, quitCompletely, reanalyzeReview, rejectReview, rescanCollection, searchKnowledge, updatePreferences, type AppSnapshot, type CloseBehavior, type EnrichmentDraft, type FolderSelection, type KnowledgePageInput, type LanPreference, type LocalePreference, type ReviewSummary } from './api';

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
  let selectedReview: ReviewSummary | null = null;
  let editDraft: EnrichmentDraft | null = null;
  let selectedCollectionId: string | null = null;
  let locale: LocalePreference = 'system';
  let lanPreference: LanPreference = 'undecided';
  let closeBehavior: CloseBehavior = 'ask';
  let automaticUpdateChecks = false;
  let closeChoiceRequired = false;
  let modelLicensesConfirmed = false;

  onMount(() => {
    const unlistenClose = listen('close-choice-required', () => { closeChoiceRequired = true; });
    connect((event) => {
      snapshot = event.snapshot;
      if (event.snapshot.model?.licenseAccepted) modelLicensesConfirmed = true;
      if (event.snapshot.preferences) {
        locale = event.snapshot.preferences.locale;
        lanPreference = event.snapshot.preferences.lanPreference;
        closeBehavior = event.snapshot.preferences.closeBehavior;
        automaticUpdateChecks = event.snapshot.preferences.automaticUpdateChecks;
      }
      if (selectedReview) {
        const currentReview = event.snapshot.reviews.find((review) => review.conceptId === selectedReview?.conceptId);
        if (!currentReview || currentReview.sourceRevision !== selectedReview.sourceRevision) {
          selectedReview = null;
          editDraft = null;
        }
      }
      if (event.snapshot.search?.status !== 'searching') actionBusy = false;
      runtimeLabel = event.snapshot.phase === 'ready' ? 'Servicios privados listos' : 'Preparando servicios privados';
    }).then((initial) => {
      snapshot = initial;
      if (initial.model?.licenseAccepted) modelLicensesConfirmed = true;
      if (initial.preferences) {
        locale = initial.preferences.locale;
        lanPreference = initial.preferences.lanPreference;
        closeBehavior = initial.preferences.closeBehavior;
        automaticUpdateChecks = initial.preferences.automaticUpdateChecks;
      }
      runtimeLabel = initial.phase === 'ready' ? 'Servicios privados listos' : runtimeLabel;
    }).catch(() => { runtimeLabel = 'Vista previa sin runtime nativo'; });
    return () => { void unlistenClose.then((unlisten) => unlisten()); };
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

  async function openReview(review: ReviewSummary) {
    selectedReview = review;
    editDraft = structuredClone(review.draft);
    actionBusy = true;
    actionMessage = 'Cargando evidencia desde la fuente local…';
    try {
      await loadReviewEvidence(review);
    } catch {
      actionMessage = 'No se pudo cargar la evidencia. La aprobación continúa bloqueada.';
      actionBusy = false;
    }
  }

  async function loadMoreEvidence() {
    if (!selectedReview || snapshot?.reviewEvidence?.nextOrdinal == null) return;
    actionBusy = true;
    try {
      await loadReviewEvidence(selectedReview, snapshot.reviewEvidence.nextOrdinal);
    } catch {
      actionMessage = 'No se pudo cargar más evidencia.';
      actionBusy = false;
    }
  }

  function evidenceIsCurrent(): boolean {
    return snapshot?.reviewEvidence?.status === 'ready'
      && snapshot.reviewEvidence.conceptId === selectedReview?.conceptId
      && snapshot.reviewEvidence.sourceRevision === selectedReview.sourceRevision;
  }

  async function decideReview(decision: 'approve' | 'reject' | 'reanalyze') {
    if (!selectedReview || (decision === 'approve' && (!editDraft || !evidenceIsCurrent()))) return;
    actionBusy = true;
    try {
      if (decision === 'approve' && editDraft) await approveReview(selectedReview.conceptId, selectedReview.sourceRevision, editDraft);
      if (decision === 'reject') await rejectReview(selectedReview.conceptId);
      if (decision === 'reanalyze') await reanalyzeReview(selectedReview.conceptId);
      actionMessage = decision === 'approve' ? 'Aprobación enviada con la versión de evidencia verificada.' : decision === 'reject' ? 'Propuesta rechazada; la fuente permanece privada.' : 'Nuevo análisis solicitado al modelo local.';
    } catch {
      actionMessage = 'La decisión no se aplicó. Actualiza la evidencia antes de reintentar.';
    } finally {
      actionBusy = false;
    }
  }

  async function openKnowledge(collectionId: string) {
    selectedCollectionId = collectionId;
    actionBusy = true;
    actionMessage = 'Inspeccionando el conocimiento publicado…';
    try {
      await loadKnowledgeBundle(collectionId);
    } catch {
      actionMessage = 'No se pudo inspeccionar esta colección.';
      actionBusy = false;
    }
  }

  async function openKnowledgePage(page: KnowledgePageInput) {
    if (!selectedCollectionId) return;
    actionBusy = true;
    try {
      await loadKnowledgePage(selectedCollectionId, page);
    } catch {
      actionMessage = 'La página cambió mientras la abrías. Actualiza la colección e inténtalo otra vez.';
      actionBusy = false;
    }
  }

  async function savePreferences(completeOnboarding = false) {
    actionBusy = true;
    try {
      await updatePreferences({ locale, lanPreference, closeBehavior, automaticUpdateChecks, completeOnboarding });
      actionMessage = completeOnboarding ? 'Configuración inicial guardada.' : 'Preferencias guardadas en este dispositivo.';
    } catch {
      actionMessage = 'No se pudieron guardar las preferencias.';
      actionBusy = false;
    }
  }

  async function applyCloseChoice(choice: 'hide' | 'quit' | 'cancel') {
    closeChoiceRequired = false;
    if (choice === 'hide') await hideToTray();
    if (choice === 'quit') await quitCompletely();
  }

  async function prepareLocalModel() {
    actionBusy = true;
    try {
      await installModels(modelLicensesConfirmed || Boolean(snapshot?.model?.licenseAccepted));
      actionMessage = 'Descarga iniciada. Puedes cancelarla sin perder los archivos ya verificados.';
    } catch {
      actionMessage = 'No se pudo iniciar la preparación del modelo.';
      actionBusy = false;
    }
  }
</script>

<svelte:head><meta name="theme-color" content="#07131f" /></svelte:head>

{#if snapshot?.preferences && snapshot.preferences.completedOnboardingVersion == null}
  <main class="onboarding">
    <div class="onboarding-mark">A</div>
    <p class="eyebrow">Primera configuración</p>
    <h1>Privado primero.<br />Compartido solo cuando tú lo decides.</h1>
    <p class="lede">Elige cómo funcionará AirWiki en este dispositivo. Puedes cambiar estas opciones más tarde en Sistema.</p>
    <div class="onboarding-steps">
      <section><span>01</span><div><h2>Idioma</h2><p>La interfaz puede seguir el idioma del sistema.</p></div><select bind:value={locale}><option value="system">Sistema</option><option value="es">Español</option><option value="en">English</option></select></section>
      <section><span>02</span><div><h2>Red local</h2><p>Permite descubrir equipos cercanos; compartir sigue requiriendo pairing y grants.</p></div><select bind:value={lanPreference}><option value="disabled">Mantener desactivada</option><option value="enabled">Activar red local</option></select></section>
      <section><span>03</span><div><h2>Al cerrar</h2><p>Ocultar mantiene inferencia, watchers, MCP y LAN activos.</p></div><select bind:value={closeBehavior}><option value="ask">Preguntar siempre</option><option value="hide_to_tray">Ocultar en la bandeja</option><option value="quit">Salir completamente</option></select></section>
      {#if snapshot.model && !snapshot.model.active}<section><span>04</span><div><h2>Modelo local</h2><p>{snapshot.model.displayName ?? 'Modelo recomendado'} · {(snapshot.model.downloadBytes / 1073741824).toFixed(1)} GiB</p></div><label class="check"><input type="checkbox" bind:checked={modelLicensesConfirmed} /> Acepto las licencias indicadas</label></section>{/if}
    </div>
    {#if snapshot.model && !snapshot.model.active}<button class="secondary onboarding-model" onclick={prepareLocalModel} disabled={actionBusy || (!modelLicensesConfirmed && !snapshot.model.licenseAccepted) || !snapshot.model.fitsAvailableDisk}>Preparar IA local</button>{/if}
    <button class="primary onboarding-action" onclick={() => savePreferences(true)} disabled={actionBusy || lanPreference === 'undecided'}>Entrar a AirWiki</button>
    {#if actionMessage}<p class="action-message" aria-live="polite">{actionMessage}</p>{/if}
  </main>
{:else}
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
              <article><div><strong>{collection.name}</strong><small>{collection.documentCount} documentos · {collection.publishedCount} publicados</small></div><div class="row-actions"><button class="text-action" onclick={() => openKnowledge(collection.id)}>Abrir conocimiento</button><button class="text-action" onclick={() => rescanCollection(collection.id)}>Analizar cambios</button></div></article>
            {/each}
          </div>
        {/if}

        {#if destination === 'library' && snapshot?.knowledge?.collectionId === selectedCollectionId}
          <div class="knowledge-workspace">
            <aside class="knowledge-tree" aria-label="Páginas de conocimiento">
              <div><strong>{snapshot.knowledge.collectionName}</strong><small>{snapshot.knowledge.concepts.length} conceptos publicados</small></div>
              <button onclick={() => openKnowledgePage({ kind: 'index' })}><BookOpen size={15} />Índice</button>
              <button onclick={() => openKnowledgePage({ kind: 'log' })}><History size={15} />Historial</button>
              {#each snapshot.knowledge.concepts as concept}
                <button onclick={() => openKnowledgePage(concept.page)} title={concept.description}><FileText size={15} /><span>{concept.title}</span></button>
              {/each}
            </aside>
            <section class="knowledge-document" aria-live="polite">
              {#if snapshot.knowledge.status === 'updating'}
                <p class="loading"><RefreshCw size={16} /> El índice se está actualizando…</p>
              {:else if snapshot.knowledgePage?.collectionId === selectedCollectionId && snapshot.knowledgePage.status === 'ready'}
                <div class="document-heading"><p class="section-label">Página OKF verificada</p><h3>{snapshot.knowledgePage.title}</h3></div>
                {#if snapshot.knowledgePage.truncated}<p class="evidence-warning">Vista parcial: la página supera el límite seguro de lectura.</p>{/if}
                <div class="knowledge-blocks">
                  {#each snapshot.knowledgePage.blocks as block}
                    {#if block.kind === 'heading'}<h4 class:minor={block.level > 2}>{block.text}</h4>
                    {:else if block.kind === 'paragraph'}<p>{block.text}</p>
                    {:else if block.kind === 'listItem'}<div class="safe-list-item"><span>{block.ordered ? '—' : '•'}</span><p>{block.text}</p></div>
                    {:else if block.kind === 'code'}<pre><code>{block.text}</code></pre>
                    {:else if block.kind === 'quote'}<blockquote>{block.text}</blockquote>
                    {:else}<hr />{/if}
                  {/each}
                </div>
              {:else if snapshot.knowledge.status === 'failed'}
                <p class="evidence-warning">La colección no pudo verificarse. No se mostrará contenido incierto.</p>
              {:else}
                <div class="review-placeholder"><BookOpen size={26} /><h3>Elige una página</h3><p>Solo se muestra contenido publicado y verificado contra su fingerprint.</p></div>
              {/if}
            </section>
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
          <div class="review-workspace">
            <aside class="review-queue" aria-label="Revisiones pendientes">
              {#each snapshot.reviews as review}
                <button class:active={selectedReview?.conceptId === review.conceptId} onclick={() => openReview(review)}>
                  <strong>{review.sourceName}</strong><small>{review.collectionName} · revisión {review.sourceRevision}</small>
                </button>
              {:else}<p class="empty">No hay propuestas pendientes. Los próximos cambios aparecerán aquí.</p>{/each}
            </aside>
            {#if selectedReview && editDraft}
              <div class="review-flow">
                <section class="review-step evidence-step" aria-labelledby="evidence-title">
                  <div class="step-heading"><span>01</span><div><p>Evidencia</p><h3 id="evidence-title">Comprueba la fuente</h3></div></div>
                  {#if snapshot.reviewEvidence?.conceptId === selectedReview.conceptId && snapshot.reviewEvidence.status === 'ready'}
                    <div class="excerpts">
                      {#each snapshot.reviewEvidence.excerpts as excerpt}
                        <article><small>{excerpt.headingOrPage || `Fragmento ${excerpt.ordinal + 1}`}</small><p>{excerpt.text}</p></article>
                      {/each}
                    </div>
                    {#if snapshot.reviewEvidence.nextOrdinal != null}<button class="secondary" onclick={loadMoreEvidence} disabled={actionBusy}>Cargar más evidencia</button>{/if}
                  {:else if snapshot.reviewEvidence?.conceptId === selectedReview.conceptId}
                    <p class="evidence-warning">La evidencia no está disponible o quedó obsoleta. La aprobación está bloqueada.</p>
                  {:else}
                    <p class="loading"><RefreshCw size={16} /> Verificando la revisión actual…</p>
                  {/if}
                </section>
                <section class="review-step proposal-step" aria-labelledby="proposal-title">
                  <div class="step-heading"><span>02</span><div><p>Propuesta de IA</p><h3 id="proposal-title">Edita antes de decidir</h3></div></div>
                  <label><span>Título</span><input bind:value={editDraft.title} maxlength="240" /></label>
                  <label><span>Descripción</span><textarea bind:value={editDraft.description} maxlength="1000" rows="3"></textarea></label>
                  <label><span>Resumen</span><textarea bind:value={editDraft.summary} maxlength="4000" rows="6"></textarea></label>
                  <label><span>Etiquetas</span><input value={editDraft.tags.join(', ')} onchange={(event) => { editDraft!.tags = event.currentTarget.value.split(',').map((tag) => tag.trim()).filter(Boolean); }} /></label>
                </section>
                <section class="review-step decision-step" aria-labelledby="decision-title">
                  <div class="step-heading"><span>03</span><div><p>Decisión humana</p><h3 id="decision-title">Define qué ocurre</h3></div></div>
                  <p>Aprobar publica la propuesta validada. Rechazar conserva la fuente y descarta solo este borrador.</p>
                  <div class="decision-actions">
                    <button class="primary" onclick={() => decideReview('approve')} disabled={actionBusy || !evidenceIsCurrent()}>Aprobar con evidencia</button>
                    <button class="secondary" onclick={() => decideReview('reanalyze')} disabled={actionBusy || !snapshot.model?.active}>Volver a analizar</button>
                    <button class="danger" onclick={() => decideReview('reject')} disabled={actionBusy}>Rechazar propuesta</button>
                  </div>
                  {#if !evidenceIsCurrent()}<small class="guardrail">Carga evidencia vigente para habilitar la aprobación.</small>{/if}
                </section>
              </div>
            {:else}
              <div class="review-placeholder"><CheckCircle2 size={26} /><h3>Elige una propuesta</h3><p>La evidencia aparece antes que cualquier acción de publicación.</p></div>
            {/if}
          </div>
        {/if}

        {#if destination === 'system' && snapshot}
          <div class="system-layout">
            <section><p class="section-label">IA local</p><h3>{snapshot.model?.displayName ?? 'Modelo recomendado'}</h3><p>{snapshot.model?.active ? 'El modelo está activo y listo para proponer metadatos.' : 'El modelo requiere preparación antes de analizar documentos.'}</p>{#if snapshot.modelInstall}<progress max={snapshot.modelInstall.totalBytes || 1} value={snapshot.modelInstall.downloaded}></progress><small>{snapshot.modelInstall.status}</small><button class="secondary" onclick={cancelModelInstall}>Cancelar descarga</button>{:else if snapshot.model && !snapshot.model.active}<label class="check license-check"><input type="checkbox" bind:checked={modelLicensesConfirmed} /> Acepto {snapshot.model.license ?? 'las licencias del modelo y sus componentes'}</label><button class="secondary" onclick={prepareLocalModel} disabled={!modelLicensesConfirmed && !snapshot.model.licenseAccepted}>Preparar modelo local</button>{/if}</section>
            <section class="settings-form"><p class="section-label">Preferencias del dispositivo</p><label><span>Idioma</span><select bind:value={locale}><option value="system">Sistema</option><option value="es">Español</option><option value="en">English</option></select></label><label><span>Red local</span><select bind:value={lanPreference}><option value="disabled">Desactivada</option><option value="enabled">Activada</option></select></label><label><span>Al cerrar</span><select bind:value={closeBehavior}><option value="ask">Preguntar</option><option value="hide_to_tray">Ocultar en bandeja</option><option value="quit">Salir completamente</option></select></label><label class="check"><input type="checkbox" bind:checked={automaticUpdateChecks} /> Buscar actualizaciones automáticamente</label><button class="primary" onclick={() => savePreferences(false)} disabled={actionBusy}>Guardar preferencias</button></section>
            <section><p class="section-label">Conectividad</p><h3>{snapshot.peers.length} equipos conocidos</h3><p>Las colecciones solo salen de este dispositivo con permisos explícitos.</p></section>
          </div>
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
{/if}
{#if closeChoiceRequired}
  <div class="modal-backdrop" role="presentation">
    <div class="close-dialog" role="dialog" aria-modal="true" aria-labelledby="close-title">
      <p class="section-label">Al cerrar AirWiki</p><h2 id="close-title">¿Mantener los servicios activos?</h2>
      <p>Ocultar conserva watchers, MCP, red local e inferencia. Salir los detiene de forma coordinada.</p>
      <div><button class="primary" onclick={() => applyCloseChoice('hide')}>Ocultar en bandeja</button><button class="danger" onclick={() => applyCloseChoice('quit')}>Salir completamente</button><button class="secondary" onclick={() => applyCloseChoice('cancel')}>Cancelar</button></div>
    </div>
  </div>
{/if}
