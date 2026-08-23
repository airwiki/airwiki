ALTER TABLE peers ADD COLUMN device_platform TEXT
    CHECK(device_platform IS NULL OR device_platform IN ('macos', 'windows'));
