-- SPDX-FileCopyrightText: 2026 Marc Seibert
--
-- SPDX-License-Identifier: AGPL-3.0-or-later

-- Forecast shows the appointments of the calendars, unless switched off
-- (ADR 0047 §10). A column with a default: no rebuild.
ALTER TABLE settings ADD COLUMN forecast_events INTEGER NOT NULL DEFAULT 1
    CHECK (forecast_events IN (0, 1));
