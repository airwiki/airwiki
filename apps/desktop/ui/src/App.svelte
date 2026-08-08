<script lang="ts">
  import { AlertTriangle, BookOpen, CheckCircle2, FileText, History, Network, RefreshCw, Search, Settings2, Sparkles } from '@lucide/svelte';
  import { listen } from '@tauri-apps/api/event';
  import { onMount } from 'svelte';
  import { addCollection, approveReview, cancelModelInstall, checkUpdates, configureFirewall, confirmPairing, connect, downloadUpdate, hideToTray, installModels, installUpdate, loadKnowledgeBundle, loadKnowledgePage, loadReviewEvidence, manageIntegration, openAdvancedFirewall, pairPeer, pickCollectionFolder, quitCompletely, reanalyzeReview, refreshAutostart, refreshConnectivity, refreshWikiHealth, rejectReview, relinkCollection, rescanCollection, revokePeer, searchKnowledge, setAutostart, setCollectionGrant, updateCollectionPolicy, updatePreferences, type AppSnapshot, type CloseBehavior, type CollectionPolicyInput, type CollectionSummary, type EnrichmentDraft, type FolderSelection, type IntegrationActionInput, type IntegrationClient, type KnowledgePageInput, type LanPreference, type LocalePreference, type ReviewSummary } from './api';
  import KnowledgeGraph from './KnowledgeGraph.svelte';

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
  let relinkSelection: FolderSelection | null = null;
  let collectionName = '';
  let editingCollectionId: string | null = null;
  let collectionPolicy: CollectionPolicyInput = { localOnly: true, peerShareable: false, allowExternalAi: false, internetPublic: false };
  let question = '';
  let includePublic = false;
  let actionMessage = '';
  let actionBusy = false;
  let selectedReview: ReviewSummary | null = null;
  let editDraft: EnrichmentDraft | null = null;
  let selectedCollectionId: string | null = null;
  let knowledgeMode: 'document' | 'graph' = 'document';
  let locale: LocalePreference = 'system';
  let lanPreference: LanPreference = 'undecided';
  let closeBehavior: CloseBehavior = 'ask';
  let automaticUpdateChecks = false;
  let closeChoiceRequired = false;
  let modelLicensesConfirmed = false;
  let autostartBusy = false;
  let autostartRequestId: string | null = null;
  let wikiHealthRequestId: string | null = null;
  let connectivityRequestId: string | null = null;
  let peerActionId: string | null = null;
  let integrationRequestId: string | null = null;
  let updaterRequestId: string | null = null;
  let confirmUpdateInstall = false;

  onMount(() => {
    const unlistenClose = '__TAURI_INTERNALS__' in window
      ? listen('close-choice-required', () => { closeChoiceRequired = true; })
      : Promise.resolve(() => {});
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
      if (event.requestId && event.requestId === autostartRequestId) {
        autostartBusy = false;
        autostartRequestId = null;
      }
      if (event.requestId && event.requestId === wikiHealthRequestId) wikiHealthRequestId = null;
      if (event.requestId && event.requestId === connectivityRequestId) connectivityRequestId = null;
      if (event.requestId && event.requestId === integrationRequestId) integrationRequestId = null;
      if (event.requestId && event.requestId === updaterRequestId) updaterRequestId = null;
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
    if (next === 'system') {
      void refreshAutostartState();
      void runConnectivityAction('refresh');
      void runIntegrationAction({ kind: 'refresh' });
    }
    if (next === 'library') void refreshHealth();
  }

  function integrationName(client: IntegrationClient): string {
    if (client === 'chatGptDesktop') return 'ChatGPT Desktop / Work';
    if (client === 'claudeDesktop') return 'Claude Desktop';
    return 'Gemini CLI';
  }

  function integrationState(status: string): string {
    const labels: Record<string, string> = {
      notInstalled: 'No detectado', available: 'Disponible', configuring: 'Configurando',
      awaitingClientApproval: 'Esperando aprobación', configured: 'Conectado',
      updateAvailable: 'Requiere actualización', conflict: 'Conflicto',
      unsupported: 'No compatible', error: 'Necesita reparación'
    };
    return labels[status] ?? status;
  }

  async function runIntegrationAction(action: IntegrationActionInput) {
    const requestId = crypto.randomUUID();
    integrationRequestId = requestId;
    try {
      await manageIntegration(requestId, action);
    } catch {
      integrationRequestId = null;
      actionMessage = 'La integración no pudo modificarse.';
    }
  }

  function updaterLabel(): string {
    const updater = snapshot?.updater;
    if (!updater) return 'Preparando el servicio de actualizaciones…';
    const labels: Record<string, string> = {
      disabled: updater.issue === 'notConfigured' ? 'Las actualizaciones internas no están configuradas en este build.' : 'El actualizador no está disponible en este build.',
      idle: 'Listo para comprobar el canal estable.',
      checking: 'Comprobando el manifiesto firmado…',
      upToDate: 'AirWiki está actualizado.',
      available: `AirWiki ${updater.version ?? ''} está disponible.`,
      downloading: `Descargando y verificando AirWiki ${updater.version ?? ''}…`,
      readyToInstall: `AirWiki ${updater.version ?? ''} está verificado y listo para instalar.`,
      installing: 'Instalando la actualización confirmada…',
      installed: 'Actualización instalada. AirWiki se cerrará de forma coordinada.'
    };
    if (updater.issue === 'offline') return 'No se pudo contactar al canal estable. AirWiki continúa funcionando normalmente.';
    if (updater.issue === 'invalidSignature') return 'La firma del paquete no es válida. La instalación fue bloqueada.';
    if (updater.issue === 'invalidManifest') return 'El manifiesto de actualización no es válido.';
    return labels[updater.status] ?? 'Estado de actualización desconocido.';
  }

  async function runUpdaterAction(action: 'check' | 'download' | 'install') {
    const requestId = crypto.randomUUID();
    updaterRequestId = requestId;
    confirmUpdateInstall = false;
    try {
      if (action === 'check') await checkUpdates(requestId);
      else if (action === 'download') await downloadUpdate(requestId);
      else await installUpdate(requestId);
    } catch {
      updaterRequestId = null;
      actionMessage = 'La operación de actualización no pudo iniciarse.';
    }
  }

  function connectivityLabel(): string {
    if (snapshot?.lanRuntime?.listener === 'listening') return 'AirWiki escucha en la red local autorizada.';
    if (snapshot?.lanRuntime?.listener === 'starting') return 'La red local se está iniciando.';
    if (snapshot?.connectivity?.networkProfile === 'public') return 'Windows clasifica la red como pública; AirWiki no abrirá el listener.';
    if (snapshot?.connectivity?.firewall === 'rulesMissing') return 'Faltan las reglas restringidas de Windows Firewall.';
    if (snapshot?.connectivity?.firewall === 'conflict' || snapshot?.connectivity?.firewall === 'legacyExposure') return 'La configuración del firewall entra en conflicto con la política segura.';
    if (snapshot?.lanRuntime?.listener === 'failed') return 'El listener local falló y permanece cerrado.';
    if (lanPreference === 'disabled') return 'La red local está desactivada por preferencia.';
    return 'Comprobando permisos, perfil de red y firewall…';
  }

  function lanStateLabel(state: string): string {
    const labels: Record<string, string> = {
      stopped: 'Detenido', starting: 'Iniciando', listening: 'Escuchando', failed: 'Falló',
      disabled: 'Desactivado', active: 'Activo'
    };
    return labels[state] ?? state;
  }

  async function runConnectivityAction(action: 'refresh' | 'install' | 'remove' | 'advanced') {
    const requestId = crypto.randomUUID();
    connectivityRequestId = requestId;
    try {
      if (action === 'refresh') await refreshConnectivity(requestId);
      if (action === 'install') await configureFirewall(requestId, true);
      if (action === 'remove') await configureFirewall(requestId, false);
      if (action === 'advanced') await openAdvancedFirewall(requestId);
    } catch {
      connectivityRequestId = null;
      actionMessage = 'La operación de conectividad no pudo comenzar.';
    }
  }

  function shortPeerId(peerId: string): string {
    return peerId.length > 18 ? `${peerId.slice(0, 9)}…${peerId.slice(-7)}` : peerId;
  }

  async function runPeerAction(peerId: string, action: 'pair' | 'accept' | 'reject' | 'revoke') {
    peerActionId = peerId;
    try {
      if (action === 'pair') await pairPeer(peerId);
      if (action === 'accept') await confirmPairing(peerId, true);
      if (action === 'reject') await confirmPairing(peerId, false);
      if (action === 'revoke') await revokePeer(peerId);
    } catch {
      actionMessage = 'La operación de confianza no se aplicó.';
    } finally {
      peerActionId = null;
    }
  }

  async function changeGrant(peerId: string, collectionId: string, granted: boolean) {
    peerActionId = peerId;
    try {
      await setCollectionGrant(peerId, collectionId, granted);
    } catch {
      actionMessage = 'El permiso de colección no se modificó.';
    } finally {
      peerActionId = null;
    }
  }

  async function refreshHealth() {
    const requestId = crypto.randomUUID();
    wikiHealthRequestId = requestId;
    try {
      await refreshWikiHealth(requestId);
    } catch {
      wikiHealthRequestId = null;
      actionMessage = 'No se pudo comprobar la salud de la biblioteca.';
    }
  }

  async function openAttentionCollection() {
    const collectionId = snapshot?.wikiHealth?.attentionCollectionId;
    if (collectionId) await openKnowledge(collectionId);
  }

  function autostartLabel(): string {
    if (snapshot?.autostart === 'enabled') return 'AirWiki se inicia al entrar en tu sesión.';
    if (snapshot?.autostart === 'disabled') return 'El inicio automático está desactivado.';
    if (snapshot?.autostart === 'requiresApproval') return 'El sistema necesita tu aprobación para activar el inicio automático.';
    if (snapshot?.autostart === 'conflict') return 'Existe otra entrada de inicio para AirWiki. No se modificará automáticamente.';
    if (snapshot?.autostart === 'unsupported') return 'Esta instalación no admite inicio automático.';
    return 'Comprobando el estado del sistema…';
  }

  async function refreshAutostartState() {
    autostartBusy = true;
    const requestId = crypto.randomUUID();
    autostartRequestId = requestId;
    try {
      await refreshAutostart(requestId);
    } catch {
      actionMessage = 'No se pudo comprobar el inicio automático.';
      autostartBusy = false;
      autostartRequestId = null;
    }
  }

  async function changeAutostart(enabled: boolean) {
    autostartBusy = true;
    const requestId = crypto.randomUUID();
    autostartRequestId = requestId;
    try {
      await setAutostart(requestId, enabled);
      actionMessage = enabled ? 'Solicitud de inicio automático enviada.' : 'Solicitud para desactivar el inicio automático enviada.';
    } catch {
      actionMessage = 'No se pudo cambiar el inicio automático.';
      autostartBusy = false;
      autostartRequestId = null;
    }
  }

  function nextActionLabel(): string {
    if (destination === 'library') return 'Añadir colección';
    if (destination === 'review') return snapshot?.reviews.length ? 'Revisar siguiente' : 'Sin pendientes';
    if (destination === 'search') return 'Hacer una pregunta';
    return 'Guardar cambios';
  }

  async function runNextAction() {
    if (destination === 'library') await chooseFolder();
    if (destination === 'review' && snapshot?.reviews[0]) await openReview(snapshot.reviews[0]);
    if (destination === 'search') document.querySelector<HTMLTextAreaElement>('#knowledge-question')?.focus();
    if (destination === 'system') await savePreferences(false);
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

  function collectionScanState(collectionId: string) {
    return snapshot?.collectionScans.find((scan) => scan.collectionId === collectionId)?.state ?? null;
  }

  async function scanCollection(collectionId: string) {
    try {
      await rescanCollection(collectionId);
      actionMessage = 'Análisis añadido a la cola.';
    } catch {
      actionMessage = 'No se pudo iniciar el análisis de cambios.';
    }
  }

  function editCollection(collection: CollectionSummary) {
    editingCollectionId = collection.id;
    relinkSelection = null;
    collectionPolicy = {
      localOnly: collection.localOnly,
      peerShareable: collection.peerShareable,
      allowExternalAi: collection.allowExternalAi,
      internetPublic: collection.internetPublic
    };
  }

  async function chooseRelinkFolder() {
    try {
      relinkSelection = await pickCollectionFolder();
    } catch {
      actionMessage = 'No se pudo abrir el selector de carpetas.';
    }
  }

  async function applyRelink() {
    if (!editingCollectionId || !relinkSelection) return;
    actionBusy = true;
    try {
      await relinkCollection(editingCollectionId, relinkSelection.token);
      relinkSelection = null;
      actionMessage = 'La colección quedó vinculada a la carpeta elegida y se volverá a comprobar.';
    } catch {
      relinkSelection = null;
      actionMessage = 'No se pudo volver a vincular la colección. El estado anterior permanece intacto.';
    } finally {
      actionBusy = false;
    }
  }

  async function saveCollectionPolicy() {
    if (!editingCollectionId) return;
    actionBusy = true;
    try {
      const policy = {
        ...collectionPolicy,
        localOnly: !collectionPolicy.peerShareable && !collectionPolicy.allowExternalAi && !collectionPolicy.internetPublic
      };
      await updateCollectionPolicy(editingCollectionId, policy);
      collectionPolicy = policy;
      actionMessage = 'Política de colección actualizada.';
    } catch {
      actionMessage = 'La política no se aplicó. Revisa que la combinación sea válida.';
    } finally {
      actionBusy = false;
    }
  }

  function modelInstallLabel(): string {
    const status = snapshot?.modelInstall?.status;
    if (status === 'queued') return 'Esperando turno';
    if (status === 'downloading') return 'Descargando archivos verificados';
    if (status === 'verifying') return 'Verificando integridad';
    if (status === 'extracting') return 'Preparando runtime local';
    return 'Activando el modelo';
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
    knowledgeMode = 'document';
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

  async function selectGraphPage(page: KnowledgePageInput) {
    knowledgeMode = 'document';
    await openKnowledgePage(page);
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
      <button class="primary" onclick={runNextAction} disabled={destination === 'review' && !snapshot?.reviews.length}><Sparkles size={17} />{nextActionLabel()}</button>
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

        {#if destination === 'library' && snapshot?.wikiHealth}
          <section class:attention={snapshot.wikiHealth.errorCount > 0} class="health-strip" aria-labelledby="health-title">
            <div><span class="health-signal" aria-hidden="true"></span><div><p class="section-label">Integridad publicada</p><h3 id="health-title">{snapshot.wikiHealth.status === 'failed' ? 'No se pudo completar la comprobación' : snapshot.wikiHealth.errorCount ? `${snapshot.wikiHealth.errorCount} errores requieren decisión` : snapshot.wikiHealth.warningCount ? `${snapshot.wikiHealth.warningCount} advertencias para revisar` : 'El conocimiento publicado es coherente'}</h3><small>{snapshot.wikiHealth.updatingCount ? `${snapshot.wikiHealth.updatingCount} colecciones todavía se están actualizando.` : 'SQLite y los artefactos OKF fueron comparados sin elegir silenciosamente un lado.'}</small></div></div>
            <div class="row-actions">{#if snapshot.wikiHealth.attentionCollectionId}<button class="secondary" onclick={openAttentionCollection}>Examinar colección</button>{/if}<button class="text-action" onclick={refreshHealth} disabled={wikiHealthRequestId !== null}>{wikiHealthRequestId ? 'Comprobando…' : 'Comprobar ahora'}</button></div>
          </section>
        {/if}

        {#if destination === 'library' && snapshot?.collections.length}
          <div class="records" aria-label="Colecciones">
            {#each snapshot.collections as collection}
              <article><div><strong>{collection.name}</strong><small>{collection.documentCount} documentos · {collection.publishedCount} publicados{#if collectionScanState(collection.id)} · {collectionScanState(collection.id) === 'queued' ? 'en cola' : 'analizando'}{/if}</small></div><div class="row-actions"><button class="text-action" onclick={() => openKnowledge(collection.id)}>Abrir conocimiento</button><button class="text-action" onclick={() => editCollection(collection)}>Configurar</button><button class="text-action" onclick={() => scanCollection(collection.id)} disabled={collectionScanState(collection.id) !== null}>{collectionScanState(collection.id) ? 'Procesando…' : 'Analizar cambios'}</button></div></article>
            {/each}
          </div>
        {/if}

        {#if destination === 'library' && editingCollectionId}
          <section class="collection-settings" aria-labelledby="collection-settings-title">
            <div class="settings-heading"><div><p class="section-label">Política de colección</p><h3 id="collection-settings-title">{snapshot?.collections.find((collection) => collection.id === editingCollectionId)?.name}</h3></div><button class="text-action" onclick={() => { editingCollectionId = null; relinkSelection = null; }}>Cerrar</button></div>
            <div class="policy-state"><strong>{!collectionPolicy.peerShareable && !collectionPolicy.allowExternalAi && !collectionPolicy.internetPublic ? 'Solo local' : 'Tiene salidas habilitadas'}</strong><span>“Solo local” se deriva automáticamente de los tres permisos de salida.</span></div>
            <div class="policy-grid">
              <label class="check"><input type="checkbox" bind:checked={collectionPolicy.peerShareable} /> Disponible para grants LAN explícitos</label>
              <label class="check"><input type="checkbox" bind:checked={collectionPolicy.allowExternalAi} /> Permitir IA externa autorizada</label>
              <label class="check"><input type="checkbox" bind:checked={collectionPolicy.internetPublic} /> Publicar en índices públicos configurados</label>
            </div>
            <p class="guardrail">AirWiki validará la combinación y fallará cerrado si una opción contradice otra.</p>
            <div class="collection-settings-actions"><button class="primary" onclick={saveCollectionPolicy} disabled={actionBusy}>Guardar política</button><button class="secondary" onclick={chooseRelinkFolder}>Elegir nueva carpeta</button>{#if relinkSelection}<span>{relinkSelection.displayPath}</span><button class="secondary" onclick={applyRelink} disabled={actionBusy}>Confirmar vínculo</button>{/if}</div>
          </section>
        {/if}

        {#if destination === 'library' && snapshot?.sourceIssues.length}
          <section class="source-issues" aria-labelledby="source-issues-title">
            <div><AlertTriangle size={18} aria-hidden="true" /><div><h3 id="source-issues-title">Fuentes que necesitan atención</h3><p>AirWiki conserva el último estado seguro y no publica contenido incierto.</p></div></div>
            {#each snapshot.sourceIssues as issue}
              <article><strong>{issue.sourceName}</strong><span>{issue.collectionName}</span><code>{issue.code}</code></article>
            {/each}
          </section>
        {/if}

        {#if destination === 'library' && snapshot?.knowledge?.collectionId === selectedCollectionId}
          <div class="knowledge-workspace">
            <aside class="knowledge-tree" aria-label="Páginas de conocimiento">
              <div><strong>{snapshot.knowledge.collectionName}</strong><small>{snapshot.knowledge.concepts.length} conceptos publicados</small></div>
              <button onclick={() => openKnowledgePage({ kind: 'index' })}><BookOpen size={15} />Índice</button>
              <button onclick={() => openKnowledgePage({ kind: 'log' })}><History size={15} />Historial</button>
              <button class:active={knowledgeMode === 'graph'} onclick={() => { knowledgeMode = 'graph'; }}><Network size={15} />Mapa de relaciones</button>
              {#each snapshot.knowledge.concepts as concept}
                <button onclick={() => openKnowledgePage(concept.page)} title={concept.description}><FileText size={15} /><span>{concept.title}</span></button>
              {/each}
            </aside>
            <section class="knowledge-document" aria-live="polite">
              {#if knowledgeMode === 'graph' && snapshot.knowledge.status === 'ready'}
                {#key `${snapshot.knowledge.collectionId}:${snapshot.knowledge.version}`}
                  <KnowledgeGraph bundle={snapshot.knowledge} onselect={selectGraphPage} />
                {/key}
              {:else if snapshot.knowledge.status === 'updating'}
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
                    <button class="secondary" onclick={() => decideReview('reanalyze')} disabled={actionBusy || !snapshot.model?.active || snapshot.reanalyzingReviewIds.includes(selectedReview.conceptId)}>{snapshot.reanalyzingReviewIds.includes(selectedReview.conceptId) ? 'Analizando…' : 'Volver a analizar'}</button>
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
            <section><p class="section-label">IA local</p><h3>{snapshot.model?.displayName ?? 'Modelo recomendado'}</h3><p>{snapshot.model?.active ? 'El modelo está activo y listo para proponer metadatos.' : 'El modelo requiere preparación antes de analizar documentos.'}</p>{#if snapshot.modelInstall}<progress max={snapshot.modelInstall.totalBytes || 1} value={snapshot.modelInstall.downloaded}></progress><small>{modelInstallLabel()}</small><button class="secondary" onclick={cancelModelInstall}>Cancelar preparación</button>{:else if snapshot.model && !snapshot.model.active}<label class="check license-check"><input type="checkbox" bind:checked={modelLicensesConfirmed} /> Acepto {snapshot.model.license ?? 'las licencias del modelo y sus componentes'}</label><button class="secondary" onclick={prepareLocalModel} disabled={!modelLicensesConfirmed && !snapshot.model.licenseAccepted}>Preparar modelo local</button>{/if}</section>
            <section class="settings-form"><p class="section-label">Preferencias del dispositivo</p><label><span>Idioma</span><select bind:value={locale}><option value="system">Sistema</option><option value="es">Español</option><option value="en">English</option></select></label><label><span>Red local</span><select bind:value={lanPreference}><option value="disabled">Desactivada</option><option value="enabled">Activada</option></select></label><label><span>Al cerrar</span><select bind:value={closeBehavior}><option value="ask">Preguntar</option><option value="hide_to_tray">Ocultar en bandeja</option><option value="quit">Salir completamente</option></select></label><label class="check"><input type="checkbox" bind:checked={automaticUpdateChecks} /> Buscar actualizaciones automáticamente</label><button class="primary" onclick={() => savePreferences(false)} disabled={actionBusy}>Guardar preferencias</button></section>
            <section class="updater-section" aria-live="polite"><div class="settings-heading"><div><p class="section-label">Actualizaciones</p><h3>Canal estable firmado</h3></div>{#if snapshot.updater?.status !== 'disabled'}<button class="text-action" onclick={() => runUpdaterAction('check')} disabled={updaterRequestId !== null || snapshot.updater?.status === 'checking' || snapshot.updater?.status === 'downloading' || snapshot.updater?.status === 'installing'}>Comprobar</button>{/if}</div><p>{updaterLabel()}</p>{#if snapshot.updater?.releaseNotes}<div class="release-notes"><small>Novedades verificadas</small><p>{snapshot.updater.releaseNotes}</p></div>{/if}<div class="row-actions">{#if snapshot.updater?.status === 'available'}<button class="secondary" onclick={() => runUpdaterAction('download')} disabled={updaterRequestId !== null}>Descargar y verificar</button>{:else if snapshot.updater?.status === 'readyToInstall' && !confirmUpdateInstall}<button class="primary" onclick={() => { confirmUpdateInstall = true; }}>Instalar actualización</button>{:else if snapshot.updater?.status === 'readyToInstall' && confirmUpdateInstall}<div class="install-confirmation" role="alert"><p>AirWiki cerrará los servicios locales y aplicará la versión {snapshot.updater.version}. Tus datos y modelos permanecen en sus ubicaciones actuales.</p><button class="primary" onclick={() => runUpdaterAction('install')} disabled={updaterRequestId !== null}>Confirmar e instalar</button><button class="secondary" onclick={() => { confirmUpdateInstall = false; }} disabled={updaterRequestId !== null}>Cancelar</button></div>{:else if snapshot.updater?.retryable}<button class="secondary" onclick={() => runUpdaterAction('check')} disabled={updaterRequestId !== null}>Reintentar</button>{/if}</div><small>No se envían identificadores del dispositivo y un fallo de red no bloquea el uso normal.</small></section>
            <section><p class="section-label">Inicio de sesión</p><h3>Inicio automático</h3><p>{autostartLabel()}</p><div class="row-actions">{#if snapshot.autostart === 'enabled'}<button class="secondary" onclick={() => changeAutostart(false)} disabled={autostartBusy}>Desactivar</button>{:else if snapshot.autostart !== 'unsupported' && snapshot.autostart !== 'conflict'}<button class="secondary" onclick={() => changeAutostart(true)} disabled={autostartBusy}>{autostartBusy ? 'Comprobando…' : 'Activar'}</button>{/if}<button class="text-action" onclick={refreshAutostartState} disabled={autostartBusy}>Actualizar estado</button></div></section>
            <section class="connectivity-section"><p class="section-label">Conectividad</p><h3>{snapshot.peers.length} equipos conocidos</h3><p>{connectivityLabel()}</p>{#if snapshot.lanRuntime}<dl><div><dt>Listener</dt><dd>{lanStateLabel(snapshot.lanRuntime.listener)}</dd></div><div><dt>Descubrimiento</dt><dd>{lanStateLabel(snapshot.lanRuntime.discovery)}</dd></div><div><dt>Interfaces</dt><dd>{snapshot.lanRuntime.addressCount}</dd></div></dl>{/if}<div class="row-actions"><button class="secondary" onclick={() => runConnectivityAction('refresh')} disabled={connectivityRequestId !== null}>{connectivityRequestId ? 'Comprobando…' : 'Comprobar'}</button>{#if snapshot.connectivity?.firewallHelper === 'verified' && snapshot.connectivity.firewall !== 'ready' && snapshot.connectivity.firewall !== 'notApplicable'}<button class="secondary" onclick={() => runConnectivityAction('install')} disabled={connectivityRequestId !== null || lanPreference !== 'enabled'}>Configurar firewall</button>{/if}{#if snapshot.connectivity?.firewall === 'ready'}<button class="text-action" onclick={() => runConnectivityAction('remove')} disabled={connectivityRequestId !== null}>Quitar reglas</button>{/if}{#if snapshot.connectivity?.firewall === 'conflict' || snapshot.connectivity?.firewall === 'legacyExposure'}<button class="text-action" onclick={() => runConnectivityAction('advanced')} disabled={connectivityRequestId !== null}>Abrir configuración avanzada</button>{/if}</div><small>Compartir sigue requiriendo pairing y grants por colección.</small></section>
            <section class="peer-trust"><p class="section-label">Equipos y permisos</p><h3>Confianza explícita</h3><p>Cada equipo se verifica con seis palabras. Después eliges qué colecciones puede consultar.</p><div class="peer-list">{#each snapshot.peers as peer}<article><div class="peer-heading"><div><strong>{peer.deviceName ?? 'Equipo cercano'}</strong><code title={peer.peerId}>{shortPeerId(peer.peerId)}</code></div><span class:verified={peer.trust === 'trusted'}>{peer.trust === 'trusted' ? 'Verificado' : peer.trust === 'blocked' ? 'Revocado' : peer.activity === 'pairing' ? 'Verificando' : 'Sin verificar'}</span></div>{#if peer.sasWords}<div class="sas" aria-label="Código de verificación"><small>Comprueba estas palabras en ambos equipos</small><strong>{peer.sasWords.join(' · ')}</strong><div><button class="primary" onclick={() => runPeerAction(peer.peerId, 'accept')} disabled={peerActionId === peer.peerId}>Coinciden</button><button class="danger" onclick={() => runPeerAction(peer.peerId, 'reject')} disabled={peerActionId === peer.peerId}>No coinciden</button></div></div>{:else if peer.trust === 'unpaired'}<button class="secondary" onclick={() => runPeerAction(peer.peerId, 'pair')} disabled={peerActionId === peer.peerId || peer.activity === 'notObserved'}>Verificar equipo</button>{:else if peer.trust === 'trusted'}<div class="grant-list">{#each snapshot.collections.filter((collection) => collection.peerShareable) as collection}<label class="check"><input type="checkbox" checked={peer.grantedCollectionIds.includes(collection.id)} onchange={(event) => changeGrant(peer.peerId, collection.id, event.currentTarget.checked)} disabled={peerActionId === peer.peerId} /> {collection.name}</label>{:else}<small>Activa “grants LAN” en una colección para poder compartirla.</small>{/each}</div><button class="danger" onclick={() => runPeerAction(peer.peerId, 'revoke')} disabled={peerActionId === peer.peerId}>Revocar confianza</button>{/if}</article>{:else}<p class="empty">No hay equipos descubiertos. AirWiki no comparte nada hasta que verifiques uno.</p>{/each}</div></section>
            <section class="integrations-section"><div class="settings-heading"><div><p class="section-label">Integraciones</p><h3>Clientes de IA</h3></div><button class="text-action" onclick={() => runIntegrationAction({ kind: 'refresh' })} disabled={integrationRequestId !== null}>Actualizar</button></div><p>AirWiki instala un puente de solo lectura hacia el endpoint MCP local.</p>{#if snapshot.integrations?.externalAiCollectionCount}<p class="evidence-warning">{snapshot.integrations.externalAiCollectionCount} colecciones permiten IA externa. Cada búsqueda seguirá revalidando la política.</p>{/if}<div class="integration-list">{#each snapshot.integrations?.integrations ?? [] as integration}<article><div><strong>{integrationName(integration.client)}</strong><small>{integrationState(integration.status)}{#if integration.detectedVersion} · {integration.detectedVersion}{/if}{#if integration.restartRequired} · reinicio requerido{/if}</small></div><div class="row-actions">{#if integration.status === 'available' || integration.status === 'updateAvailable'}<button class="secondary" onclick={() => runIntegrationAction({ kind: 'connect', client: integration.client })} disabled={integrationRequestId !== null}>{integration.status === 'updateAvailable' ? 'Actualizar puente' : 'Conectar'}</button>{:else if integration.status === 'configured'}<button class="danger" onclick={() => runIntegrationAction({ kind: 'disconnect', client: integration.client })} disabled={integrationRequestId !== null}>Desconectar</button>{:else if integration.status === 'awaitingClientApproval' && integration.client === 'claudeDesktop'}<button class="secondary" onclick={() => runIntegrationAction({ kind: 'openClaudeSettings' })} disabled={integrationRequestId !== null}>Abrir Claude</button><button class="text-action" onclick={() => runIntegrationAction({ kind: 'confirmClaudeInstalled' })} disabled={integrationRequestId !== null}>Ya lo aprobé</button>{/if}</div></article>{:else}<p class="empty">Comprueba el sistema para detectar clientes compatibles.</p>{/each}</div></section>
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
