# SKILL.md — EVEZ Plugin Manifest v2
id: metarom
name: MetaROM Memory Layer
version: 0.1.0
schema: 2

runtime:
  port: 8003
  base_url: http://localhost:8003
  health_endpoint: /health
  skills_endpoint: /skills

capabilities:
  - memory_write
  - memory_read
  - memory_search
  - rom_snapshot
  - rom_replay

fire_events:
  - FIRE_PLUGIN_ACTIVATED
  - FIRE_PLUGIN_DEACTIVATED
  - FIRE_PLUGIN_ERROR
  - FIRE_MEMORY_WRITTEN
  - FIRE_MEMORY_RETRIEVED
  - FIRE_ROM_SNAPSHOT

dependencies:
  - evez-os

auth:
  type: api_key
  header: X-EVEZ-API-KEY

termux:
  start_cmd: "python3 -m uvicorn api:app --port 8003"
  pm2_name: metarom
