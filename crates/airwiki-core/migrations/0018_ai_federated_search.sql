-- Public query egress is an application-specific consent and is never inherited
-- from an existing capability.
ALTER TABLE application_capabilities
    ADD COLUMN public_search_enabled INTEGER NOT NULL DEFAULT 0
    CHECK(public_search_enabled IN (0, 1));

-- Existing LAN grants retain device-to-device access but remain legacy for
-- searches initiated by AI applications until a person confirms the new scope.
ALTER TABLE grants
    ADD COLUMN receiver_ai_consent_version INTEGER NOT NULL DEFAULT 0
    CHECK(receiver_ai_consent_version IN (0, 1));
