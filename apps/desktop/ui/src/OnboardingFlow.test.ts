import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import axe from 'axe-core';
import { afterEach, describe, expect, it, vi } from 'vitest';
import OnboardingFlow from './OnboardingFlow.svelte';
import { readySnapshot } from './test/fixtures';

function onboardingProps() {
  const snapshot = readySnapshot();
  snapshot.wikis = [];
  return {
    snapshot,
    locale: 'es' as const,
    modelLicensesConfirmed: false,
    actionBusy: false,
    actionMessage: '',
    onpickfolder: vi.fn(async () => ({ token: 'synthetic-folder-token', displayName: 'Apuntes' })),
    oncreatewiki: vi.fn(async () => undefined),
    onprepare: vi.fn(),
    onopenmodelsettings: vi.fn(),
    onfinish: vi.fn()
  };
}

describe('OnboardingFlow', () => {
  afterEach(cleanup);

  it('moves focus with each step and makes postponing the first Wiki explicit', async () => {
    const { container } = render(OnboardingFlow, onboardingProps());

    await fireEvent.click(screen.getByRole('button', { name: 'Continuar' }));
    const folderHeading = screen.getByRole('heading', { name: 'Agregar tu primera wiki' });
    await waitFor(() => expect(folderHeading).toHaveFocus());
    expect(screen.getByRole('button', { name: 'Continuar' })).toBeDisabled();

    await fireEvent.click(screen.getByRole('button', { name: 'Continuar sin una carpeta' }));
    expect(screen.getByRole('button', { name: 'Continuar' })).toBeEnabled();
    await fireEvent.click(screen.getByRole('button', { name: 'Continuar' }));
    await waitFor(() => expect(screen.getByRole('heading', { name: 'AirWiki está listo' })).toHaveFocus());
    expect(screen.getByText(/Agrega una carpeta desde el estado vacío/)).toBeInTheDocument();
    expect(screen.getByText('Siguiente paso: preparar la búsqueda local')).toBeInTheDocument();

    const report = await axe.run(container, { rules: { 'color-contrast': { enabled: false } } });
    expect(report.violations.filter((violation) => ['critical', 'serious'].includes(violation.impact ?? ''))).toEqual([]);
  });

  it('opens local AI settings from the recovery action when setup was deferred', async () => {
    const props = onboardingProps();
    render(OnboardingFlow, props);

    await fireEvent.click(screen.getByRole('button', { name: 'Continuar' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Continuar sin una carpeta' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Continuar' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Abrir configuración de IA local' }));

    expect(props.onopenmodelsettings).toHaveBeenCalledTimes(1);
  });

  it('shows a retryable local AI failure without claiming search is ready', async () => {
    const props = onboardingProps();
    props.snapshot.model = {
      stateSequence: 1,
      profile: 'automatic',
      recommendedModelId: 'synthetic-model',
      displayName: 'Synthetic local model',
      recommendationReason: null,
      active: false,
      activeModelId: null,
      installed: false,
      degraded: false,
      issues: [],
      pendingModelId: null,
      downloadBytes: 1073741824,
      requiredFreeBytes: 2147483648,
      fitsAvailableDisk: true,
      licenseAccepted: true,
      license: null,
      licenseUrl: null,
      revision: null
    };
    props.actionMessage = 'No se pudo preparar la IA local.';
    render(OnboardingFlow, props);

    await fireEvent.click(screen.getByRole('button', { name: 'Continuar' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Continuar sin una carpeta' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Continuar' }));

    expect(screen.getByRole('alert')).toHaveTextContent('No se pudo preparar la IA local.');
    expect(screen.getAllByText('No se pudo preparar la IA local.')).toHaveLength(1);
    await fireEvent.click(screen.getByRole('button', { name: 'Reintentar la preparación de la IA local' }));
    expect(props.onprepare).toHaveBeenCalledTimes(1);
  });

  it('does not promise local AI setup when the device cannot install a supported model', async () => {
    const props = onboardingProps();
    props.snapshot.hardware = { ...props.snapshot.hardware!, canInstall: false, issues: ['unsupported_hardware'] };
    render(OnboardingFlow, props);

    await fireEvent.click(screen.getByRole('button', { name: 'Continuar' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Continuar sin una carpeta' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Continuar' }));

    expect(screen.getByText('La búsqueda local no está disponible en este equipo')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Abrir configuración de IA local' })).not.toBeInTheDocument();
    expect(screen.getByText('No disponible en este equipo')).toBeInTheDocument();
  });

  it('disables the recovery action while onboarding completion is saving', async () => {
    const props = onboardingProps();
    props.actionBusy = true;
    render(OnboardingFlow, props);

    await fireEvent.click(screen.getByRole('button', { name: 'Continuar' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Continuar sin una carpeta' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Continuar' }));

    expect(screen.getByRole('button', { name: 'Abrir configuración de IA local' })).toBeDisabled();
  });

  it('creates the first folder Wiki from the setup flow', async () => {
    const props = onboardingProps();
    render(OnboardingFlow, props);
    await fireEvent.click(screen.getByRole('button', { name: 'Continuar' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Elegir carpeta…' }));

    expect(await screen.findByRole('textbox', { name: 'Nombre de la wiki' })).toHaveValue('Apuntes');
    await fireEvent.click(screen.getByRole('button', { name: 'Crear wiki' }));

    await waitFor(() => expect(props.oncreatewiki).toHaveBeenCalledWith('Apuntes', 'synthetic-folder-token', true));
    expect(screen.getByText('La primera carpeta ya está vinculada.')).toBeInTheDocument();
  });
});
