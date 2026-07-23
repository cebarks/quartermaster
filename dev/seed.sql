-- Dev seed data for Quartermaster
-- Run via: just dev-seed
-- All seed users have password: devdevdev

PRAGMA foreign_keys = ON;

-- Wipe all seed-relevant tables (FK-safe order: children before parents)
DELETE FROM mod_request_votes;
DELETE FROM mod_requests;
DELETE FROM raid_kills;
DELETE FROM raid_snapshots;
DELETE FROM raids;
DELETE FROM pending_operations;
DELETE FROM installed_files;
DELETE FROM mod_dependencies;
DELETE FROM installed_mods;
DELETE FROM invite_codes;
DELETE FROM password_reset_tokens;
DELETE FROM users;

-- ============================================================
-- Users (password for all: devdevdev)
-- ============================================================
INSERT INTO users (id, username, spt_profile_id, password_hash, role, disabled, created_at, password_changed_at) VALUES
  (1, 'admin', NULL,
   '$argon2id$v=19$m=19456,t=2,p=1$no1XTJYpj+UMVT6a8EhiVQ$xC9kwrT5TO/HkhWA89Fyme8FiaEOgjmgaPfQimO++gU',
   'admin', 0, '2026-01-15 10:00:00', '2026-01-15 10:00:00'),
  (2, 'ModeratorMike', NULL,
   '$argon2id$v=19$m=19456,t=2,p=1$no1XTJYpj+UMVT6a8EhiVQ$xC9kwrT5TO/HkhWA89Fyme8FiaEOgjmgaPfQimO++gU',
   'moderator', 0, '2026-02-01 14:30:00', '2026-02-01 14:30:00'),
  (3, 'TarkovChad', NULL,
   '$argon2id$v=19$m=19456,t=2,p=1$no1XTJYpj+UMVT6a8EhiVQ$xC9kwrT5TO/HkhWA89Fyme8FiaEOgjmgaPfQimO++gU',
   'player', 0, '2026-02-10 08:15:00', '2026-02-10 08:15:00'),
  (4, 'LootGoblin', NULL,
   '$argon2id$v=19$m=19456,t=2,p=1$no1XTJYpj+UMVT6a8EhiVQ$xC9kwrT5TO/HkhWA89Fyme8FiaEOgjmgaPfQimO++gU',
   'player', 0, '2026-03-05 19:00:00', '2026-03-05 19:00:00'),
  (5, 'ExtractCamper', NULL,
   '$argon2id$v=19$m=19456,t=2,p=1$no1XTJYpj+UMVT6a8EhiVQ$xC9kwrT5TO/HkhWA89Fyme8FiaEOgjmgaPfQimO++gU',
   'player', 0, '2026-03-20 12:00:00', '2026-03-20 12:00:00'),
  (6, 'ProfileOnlyUser', 'profile-only-123', NULL,
   'player', 0, '2026-04-01 10:00:00', NULL);

-- ============================================================
-- Installed mods
-- ============================================================
INSERT INTO installed_mods (id, forge_mod_id, forge_version_id, name, slug, version, installed_at, updated_at, disabled) VALUES
  (1, 2326, 8801, 'Fika Server',     'fika-server',     '2.3.1', '2026-01-15 12:00:00', '2026-04-01 09:00:00', 0),
  (2, 2357, 8802, 'Fika Client',     'fika-client',     '2.3.1', '2026-01-15 12:00:00', '2026-04-01 09:00:00', 0),
  (3, 1062, 7750, 'Server Value Modifier', 'svm',       '1.5.8', '2026-02-20 15:00:00', NULL, 0),
  (4, 1119, 7890, 'Amands Graphics', 'amands-graphics', '1.4.2', '2026-03-01 10:00:00', NULL, 0),
  (5, 1055, 7200, 'That''s Lit',     'thats-lit',       '1.3.0', '2026-03-10 14:00:00', NULL, 1);

-- ============================================================
-- Installed files
-- ============================================================
INSERT INTO installed_files (id, mod_id, file_path, file_hash, file_size, source) VALUES
  -- Fika Server (mod 1) — server mod
  (1,  1, 'SPT/user/mods/fika-server/package.json',       'a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2', 1240, 'archive'),
  (2,  1, 'SPT/user/mods/fika-server/src/mod.js',         'b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3', 45200, 'archive'),
  (3,  1, 'SPT/user/mods/fika-server/src/controllers.js', 'c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4', 22800, 'archive'),
  (4,  1, 'SPT/user/mods/fika-server/config/config.json', 'd4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5', 890, 'runtime'),
  -- Fika Client (mod 2) — client mod
  (5,  2, 'BepInEx/plugins/Fika.Core.dll',                'e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6', 524288, 'archive'),
  (6,  2, 'BepInEx/plugins/Fika.Dedicated.dll',           'f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1', 131072, 'archive'),
  -- SVM (mod 3) — server mod
  (7,  3, 'SPT/user/mods/svm/package.json',               'aabbccdd11223344aabbccdd11223344aabbccdd11223344aabbccdd11223344', 980, 'archive'),
  (8,  3, 'SPT/user/mods/svm/src/mod.js',                 'bbccddee22334455bbccddee22334455bbccddee22334455bbccddee22334455', 38400, 'archive'),
  (9,  3, 'SPT/user/mods/svm/config/config.json',         'ccddeeff33445566ccddeeff33445566ccddeeff33445566ccddeeff33445566', 4200, 'runtime'),
  -- Amands Graphics (mod 4) — client mod
  (10, 4, 'BepInEx/plugins/AmandsGraphics.dll',           'ddeeff0044556677ddeeff0044556677ddeeff0044556677ddeeff0044556677', 262144, 'archive'),
  (11, 4, 'BepInEx/config/AmandsGraphics.cfg',            'eeff001155667788eeff001155667788eeff001155667788eeff001155667788', 3200, 'runtime'),
  -- That's Lit (mod 5, disabled) — hybrid
  (12, 5, 'SPT/user/mods/thats-lit/package.json',         '11223344aabbccdd11223344aabbccdd11223344aabbccdd11223344aabbccdd', 870, 'archive'),
  (13, 5, 'SPT/user/mods/thats-lit/src/mod.js',           '22334455bbccddee22334455bbccddee22334455bbccddee22334455bbccddee', 15600, 'archive'),
  (14, 5, 'BepInEx/plugins/ThatsLit.dll',                 '33445566ccddeeff33445566ccddeeff33445566ccddeeff33445566ccddeeff', 196608, 'archive');

-- ============================================================
-- Mod dependencies
-- ============================================================
DELETE FROM mod_dependencies;

INSERT INTO mod_dependencies (id, mod_id, depends_on_mod_id, depends_on_forge_id, depends_on_name, version_constraint) VALUES
  (1, 2, 1, 2326, 'Fika Server', '>=2.3.0'),
  (2, 5, 2, 2357, 'Fika Client', NULL),
  (3, 5, 1, 2326, 'Fika Server', NULL);

-- ============================================================
-- Raids (25 completed raids across 3 players)
-- spt_profile_id uses placeholder values until real profiles are linked
-- ============================================================
INSERT INTO raids (id, user_id, spt_profile_id, server_id, player_side, faction, map, time_variant, started_at, ended_at, play_time_seconds, exit_status, exit_name, killer_id, killer_aid, xp_before, xp_after, level_before, level_after, victim_count_before) VALUES
  -- TarkovChad (user 3) — 10 raids, good survival rate
  (1,  3, 'placeholder_3', 'srv_001', 'Pmc',   'Usec',  'Customs',      'day',   '2026-04-01 18:00:00', '2026-04-01 18:35:00', 2100, 'Survived',        'EXFIL_ZB-1012', NULL, NULL, 125000, 131500, 28, 28, 45),
  (2,  3, 'placeholder_3', 'srv_002', 'Pmc',   'Usec',  'Interchange',  'day',   '2026-04-02 20:00:00', '2026-04-02 20:42:00', 2520, 'Survived',        'EXFIL_PP',       NULL, NULL, 131500, 139200, 28, 29, 48),
  (3,  3, 'placeholder_3', 'srv_003', 'Pmc',   'Usec',  'Lighthouse',   'day',   '2026-04-03 14:00:00', '2026-04-03 14:12:00', 720,  'Killed',          NULL,             'bot_sniper', NULL, 139200, 139200, 29, 29, 50),
  (4,  3, 'placeholder_3', 'srv_004', 'Pmc',   'Usec',  'Woods',        'day',   '2026-04-04 16:00:00', '2026-04-04 16:45:00', 2700, 'Survived',        'EXFIL_Outskirts', NULL, NULL, 139200, 147800, 29, 30, 50),
  (5,  3, 'placeholder_3', 'srv_005', 'Savage', NULL,    'Factory',      'night', '2026-04-05 22:00:00', '2026-04-05 22:08:00', 480,  'Survived',        'EXFIL_Gate3',    NULL, NULL, 0, 0, 30, 30, 52),
  (6,  3, 'placeholder_3', 'srv_006', 'Pmc',   'Usec',  'Customs',      'night', '2026-04-06 23:00:00', '2026-04-06 23:28:00', 1680, 'Survived',        'EXFIL_Crossroads', NULL, NULL, 147800, 153400, 30, 30, 52),
  (7,  3, 'placeholder_3', 'srv_007', 'Pmc',   'Usec',  'Shoreline',    'day',   '2026-04-07 10:00:00', '2026-04-07 10:22:00', 1320, 'Killed',          NULL,             'Reshala', NULL, 153400, 153400, 30, 30, 55),
  (8,  3, 'placeholder_3', 'srv_008', 'Pmc',   'Usec',  'Reserve',      'day',   '2026-04-08 15:00:00', '2026-04-08 15:50:00', 3000, 'Survived',        'EXFIL_Bunker',   NULL, NULL, 153400, 162100, 30, 31, 55),
  (9,  3, 'placeholder_3', 'srv_009', 'Pmc',   'Usec',  'Streets',      'day',   '2026-04-09 19:00:00', '2026-04-09 19:15:00', 900,  'MissingInAction', NULL,             NULL, NULL, 162100, 162100, 31, 31, 58),
  (10, 3, 'placeholder_3', 'srv_010', 'Pmc',   'Usec',  'Customs',      'day',   '2026-04-10 17:00:00', '2026-04-10 17:40:00', 2400, 'Survived',        'EXFIL_ZB-1012', NULL, NULL, 162100, 170000, 31, 32, 58),
  -- LootGoblin (user 4) — 10 raids, average player
  (11, 4, 'placeholder_4', 'srv_011', 'Pmc',   'Bear',  'Customs',      'day',   '2026-04-01 19:00:00', '2026-04-01 19:25:00', 1500, 'Survived',        'EXFIL_ZB-1012', NULL, NULL, 45000, 49200, 15, 15, 12),
  (12, 4, 'placeholder_4', 'srv_012', 'Pmc',   'Bear',  'Factory',      'day',   '2026-04-02 21:00:00', '2026-04-02 21:05:00', 300,  'Killed',          NULL,             'Tagilla', NULL, 49200, 49200, 15, 15, 14),
  (13, 4, 'placeholder_4', 'srv_013', 'Savage', NULL,    'Interchange',  'day',   '2026-04-03 12:00:00', '2026-04-03 12:18:00', 1080, 'Survived',        'EXFIL_Emercom',  NULL, NULL, 0, 0, 15, 15, 14),
  (14, 4, 'placeholder_4', 'srv_014', 'Pmc',   'Bear',  'Woods',        'day',   '2026-04-04 14:00:00', '2026-04-04 14:30:00', 1800, 'Killed',          NULL,             'bot_assault', NULL, 52000, 52000, 16, 16, 14),
  (15, 4, 'placeholder_4', 'srv_015', 'Pmc',   'Bear',  'Customs',      'night', '2026-04-05 23:30:00', '2026-04-05 23:55:00', 1500, 'Survived',        'EXFIL_Crossroads', NULL, NULL, 52000, 56800, 16, 16, 16),
  (16, 4, 'placeholder_4', 'srv_016', 'Pmc',   'Bear',  'Shoreline',    'day',   '2026-04-06 11:00:00', '2026-04-06 11:35:00', 2100, 'Survived',        'EXFIL_Tunnel',   NULL, NULL, 56800, 62000, 16, 17, 16),
  (17, 4, 'placeholder_4', 'srv_017', 'Pmc',   'Bear',  'Reserve',      'day',   '2026-04-07 16:00:00', '2026-04-07 16:10:00', 600,  'Killed',          NULL,             'Glukhar', NULL, 62000, 62000, 17, 17, 18),
  (18, 4, 'placeholder_4', 'srv_018', 'Savage', NULL,    'Factory',      'night', '2026-04-08 00:00:00', '2026-04-08 00:12:00', 720,  'Survived',        'EXFIL_Gate3',    NULL, NULL, 0, 0, 17, 17, 18),
  (19, 4, 'placeholder_4', 'srv_019', 'Pmc',   'Bear',  'Lighthouse',   'day',   '2026-04-09 13:00:00', '2026-04-09 13:20:00', 1200, 'Killed',          NULL,             'bot_assault', NULL, 64000, 64000, 17, 17, 18),
  (20, 4, 'placeholder_4', 'srv_020', 'Pmc',   'Bear',  'Streets',      'day',   '2026-04-10 18:00:00', '2026-04-10 18:45:00', 2700, 'Survived',        'EXFIL_Klimov',   NULL, NULL, 64000, 71200, 17, 18, 20),
  -- ExtractCamper (user 5) — 5 raids, low survival
  (21, 5, 'placeholder_5', 'srv_021', 'Pmc',   'Usec',  'Interchange',  'day',   '2026-04-05 20:00:00', '2026-04-05 20:08:00', 480,  'Killed',          NULL,             'Killa', NULL, 18000, 18000, 8, 8, 3),
  (22, 5, 'placeholder_5', 'srv_022', 'Pmc',   'Usec',  'Factory',      'day',   '2026-04-06 21:00:00', '2026-04-06 21:03:00', 180,  'Killed',          NULL,             'bot_assault', NULL, 18000, 18000, 8, 8, 3),
  (23, 5, 'placeholder_5', 'srv_023', 'Savage', NULL,    'Customs',      'day',   '2026-04-07 15:00:00', '2026-04-07 15:20:00', 1200, 'Survived',        'EXFIL_Crossroads', NULL, NULL, 0, 0, 8, 8, 3),
  (24, 5, 'placeholder_5', 'srv_024', 'Pmc',   'Usec',  'Customs',      'night', '2026-04-08 22:00:00', '2026-04-08 22:06:00', 360,  'Killed',          NULL,             'TarkovChad', 'placeholder_3', 20000, 20000, 8, 8, 4),
  (25, 5, 'placeholder_5', 'srv_025', 'Pmc',   'Usec',  'Woods',        'day',   '2026-04-09 16:00:00', '2026-04-09 16:40:00', 2400, 'Survived',        'EXFIL_Outskirts', NULL, NULL, 20000, 25600, 8, 9, 4);

-- ============================================================
-- Raid kills (60 kills across raids)
-- ============================================================
INSERT INTO raid_kills (id, raid_id, victim_name, victim_side, victim_role, weapon, distance, body_part, kill_time) VALUES
  -- Raid 1 (TarkovChad, Customs, Survived) — 3 kills
  (1,  1, 'Reshala Guard',    'Savage', 'assault',     'AKMN',           45.2,  'Head',    '2026-04-01 18:12:00'),
  (2,  1, 'Reshala Guard',    'Savage', 'assault',     'AKMN',           38.7,  'Thorax',  '2026-04-01 18:12:05'),
  (3,  1, 'Scav',             'Savage', 'assault',     'AKMN',           12.3,  'Head',    '2026-04-01 18:25:00'),
  -- Raid 2 (TarkovChad, Interchange, Survived) — 4 kills
  (4,  2, 'Scav',             'Savage', 'assault',     'M4A1',           28.5,  'Thorax',  '2026-04-02 20:08:00'),
  (5,  2, 'Scav',             'Savage', 'assault',     'M4A1',           15.0,  'Stomach', '2026-04-02 20:15:00'),
  (6,  2, 'Scav',             'Savage', 'marksman',    'M4A1',           52.8,  'Head',    '2026-04-02 20:22:00'),
  (7,  2, 'Raider',           'Savage', 'pmcBot',      'M4A1',           8.4,   'Thorax',  '2026-04-02 20:30:00'),
  -- Raid 4 (TarkovChad, Woods, Survived) — 5 kills
  (8,  4, 'Shturman Guard',   'Savage', 'followerBully','DVL-10',       185.3,  'Head',    '2026-04-04 16:10:00'),
  (9,  4, 'Shturman Guard',   'Savage', 'followerBully','DVL-10',       192.0,  'Thorax',  '2026-04-04 16:10:30'),
  (10, 4, 'Shturman',         'Savage', 'bossKojaniy',  'DVL-10',       178.5,  'Head',    '2026-04-04 16:11:00'),
  (11, 4, 'Scav',             'Savage', 'assault',      'DVL-10',       245.0,  'Head',    '2026-04-04 16:25:00'),
  (12, 4, 'Scav',             'Savage', 'assault',      'SR-25',         67.2,  'LeftArm', '2026-04-04 16:35:00'),
  -- Raid 5 (TarkovChad, Factory Scav, Survived) — 2 kills
  (13, 5, 'Scav',             'Savage', 'assault',      'MP-153',        5.8,   'Thorax',  '2026-04-05 22:03:00'),
  (14, 5, 'Scav',             'Savage', 'assault',      'MP-153',        3.2,   'Head',    '2026-04-05 22:05:00'),
  -- Raid 6 (TarkovChad, Customs night, Survived) — 3 kills
  (15, 6, 'Scav',             'Savage', 'assault',      'AS VAL',       18.9,   'Thorax',  '2026-04-06 23:08:00'),
  (16, 6, 'Scav',             'Savage', 'assault',      'AS VAL',       22.1,   'Head',    '2026-04-06 23:15:00'),
  (17, 6, 'ExtractCamper',    'Pmc',    'pmc',          'AS VAL',       35.0,   'Head',    '2026-04-06 23:20:00'),
  -- Raid 8 (TarkovChad, Reserve, Survived) — 6 kills
  (18, 8, 'Raider',           'Savage', 'pmcBot',       'HK 416',       32.4,  'Thorax',  '2026-04-08 15:10:00'),
  (19, 8, 'Raider',           'Savage', 'pmcBot',       'HK 416',       28.0,  'Head',    '2026-04-08 15:10:05'),
  (20, 8, 'Raider',           'Savage', 'pmcBot',       'HK 416',       40.1,  'Thorax',  '2026-04-08 15:12:00'),
  (21, 8, 'Glukhar Guard',    'Savage', 'followerBully','HK 416',       15.5,  'Stomach', '2026-04-08 15:30:00'),
  (22, 8, 'Glukhar Guard',    'Savage', 'followerBully','HK 416',       12.8,  'Thorax',  '2026-04-08 15:30:10'),
  (23, 8, 'Glukhar',          'Savage', 'bossBully',    'HK 416',       18.2,  'Head',    '2026-04-08 15:31:00'),
  -- Raid 10 (TarkovChad, Customs, Survived) — 3 kills
  (24, 10, 'Scav',            'Savage', 'assault',      'MCX Spear',    55.0,  'Thorax',  '2026-04-10 17:10:00'),
  (25, 10, 'Scav',            'Savage', 'assault',      'MCX Spear',    42.3,  'Head',    '2026-04-10 17:20:00'),
  (26, 10, 'LootGoblin',      'Pmc',    'pmc',          'MCX Spear',    88.5,  'Head',    '2026-04-10 17:25:00'),
  -- Raid 11 (LootGoblin, Customs, Survived) — 2 kills
  (27, 11, 'Scav',            'Savage', 'assault',      'SKS',          30.0,   'Thorax',  '2026-04-01 19:08:00'),
  (28, 11, 'Scav',            'Savage', 'assault',      'SKS',          25.4,   'Stomach', '2026-04-01 19:15:00'),
  -- Raid 13 (LootGoblin, Interchange Scav, Survived) — 1 kill
  (29, 13, 'Scav',            'Savage', 'assault',      'TOZ-106',       4.5,   'Head',    '2026-04-03 12:10:00'),
  -- Raid 15 (LootGoblin, Customs night, Survived) — 3 kills
  (30, 15, 'Scav',            'Savage', 'assault',      'AK-74N',       22.0,   'Thorax',  '2026-04-05 23:40:00'),
  (31, 15, 'Scav',            'Savage', 'assault',      'AK-74N',       18.5,   'Head',    '2026-04-05 23:42:00'),
  (32, 15, 'Scav',            'Savage', 'marksman',     'AK-74N',       45.0,   'Thorax',  '2026-04-05 23:48:00'),
  -- Raid 16 (LootGoblin, Shoreline, Survived) — 4 kills
  (33, 16, 'Scav',            'Savage', 'assault',      'RFB',          65.0,   'Head',    '2026-04-06 11:10:00'),
  (34, 16, 'Scav',            'Savage', 'assault',      'RFB',          48.2,   'Thorax',  '2026-04-06 11:18:00'),
  (35, 16, 'Sanitar Guard',   'Savage', 'followerBully','RFB',          35.0,   'Head',    '2026-04-06 11:25:00'),
  (36, 16, 'Sanitar',         'Savage', 'bossSanitar',  'RFB',          30.8,   'Head',    '2026-04-06 11:25:30'),
  -- Raid 18 (LootGoblin, Factory Scav, Survived) — 1 kill
  (37, 18, 'Scav',            'Savage', 'assault',      'Saiga-9',       6.0,   'Thorax',  '2026-04-08 00:05:00'),
  -- Raid 20 (LootGoblin, Streets, Survived) — 5 kills
  (38, 20, 'Scav',            'Savage', 'assault',      'AK-74M',       20.0,   'Thorax',  '2026-04-10 18:08:00'),
  (39, 20, 'Scav',            'Savage', 'assault',      'AK-74M',       15.2,   'Head',    '2026-04-10 18:12:00'),
  (40, 20, 'Scav',            'Savage', 'assault',      'AK-74M',       32.0,   'Stomach', '2026-04-10 18:20:00'),
  (41, 20, 'Raider',          'Savage', 'pmcBot',       'AK-74M',       25.5,   'Thorax',  '2026-04-10 18:30:00'),
  (42, 20, 'Raider',          'Savage', 'pmcBot',       'AK-74M',       28.0,   'Head',    '2026-04-10 18:30:08'),
  -- Raid 23 (ExtractCamper, Customs Scav, Survived) — 1 kill
  (43, 23, 'Scav',            'Savage', 'assault',      'MP-43',         8.0,   'Thorax',  '2026-04-07 15:10:00'),
  -- Raid 24 (ExtractCamper killed by TarkovChad) — 0 kills (no inserts)
  -- Raid 25 (ExtractCamper, Woods, Survived) — 2 kills
  (44, 25, 'Scav',            'Savage', 'assault',      'Mosin',        120.5,  'Head',    '2026-04-09 16:15:00'),
  (45, 25, 'Scav',            'Savage', 'assault',      'Mosin',        95.0,   'Thorax',  '2026-04-09 16:28:00');

-- ============================================================
-- Mod requests
-- ============================================================
INSERT INTO mod_requests (id, user_id, forge_mod_id, mod_name, mod_slug, mod_description, fika_compatible, reason, status, resolved_by, resolved_at, resolve_comment, created_at, forge_cached_at) VALUES
  (1, 3, 850,  'Realism Mod',         'realism-mod',     'Overhauls ballistics, health, and armor systems', 'compatible',   'Would make firefights more intense',                    'pending',   NULL, NULL, NULL,                                    '2026-04-01 10:00:00', '2026-04-01 10:00:00'),
  (2, 4, 920,  'Loot Value Calculator','loot-value-calc', 'Shows item value per slot in the UI',            'compatible',   'Need this for efficient looting runs',                   'pending',   NULL, NULL, NULL,                                    '2026-04-02 14:00:00', '2026-04-02 14:00:00'),
  (3, 5, 1200, 'More Spawn Points',   'more-spawns',     'Adds additional PMC and Scav spawn locations',   'unknown',      'Current spawns are too predictable',                    'pending',   NULL, NULL, NULL,                                    '2026-04-03 09:00:00', '2026-04-03 09:00:00'),
  (4, 3, 780,  'Expanded Task List',  'expanded-tasks',  'Adds 50+ new quests from community',            'compatible',   'We need more endgame content',                          'approved',  1, '2026-04-05 12:00:00', 'Looks good, installing next maintenance window', '2026-03-20 15:00:00', '2026-03-20 15:00:00'),
  (5, 4, 640,  'Weapon Randomizer',   'weapon-random',   'Randomizes AI weapon loadouts each raid',        'incompatible', 'Would add variety to AI encounters',                    'approved',  2, '2026-04-06 10:00:00', 'Approved, but need to test Fika compat first',   '2026-03-25 11:00:00', '2026-03-25 11:00:00'),
  (6, 5, 1500, 'Full Auto Everything','full-auto',        'Makes all weapons fully automatic',              'unknown',      'Would be fun for Factory runs',                         'rejected',  1, '2026-04-04 16:00:00', 'Too unbalanced for server gameplay',             '2026-04-01 20:00:00', '2026-04-01 20:00:00');

-- ============================================================
-- Mod request votes
-- ============================================================
INSERT INTO mod_request_votes (id, request_id, user_id, upvote, comment, created_at) VALUES
  -- Realism Mod votes (request 1)
  (1,  1, 4, 1, 'Yes! Ballistics overhaul would be amazing',    '2026-04-01 12:00:00'),
  (2,  1, 5, 1, NULL,                                            '2026-04-01 15:00:00'),
  (3,  1, 2, 1, 'Looks well-maintained, +1',                    '2026-04-02 09:00:00'),
  -- Loot Value Calculator votes (request 2)
  (4,  2, 3, 1, 'Would save so much time',                      '2026-04-02 16:00:00'),
  (5,  2, 5, 0, 'Prefer to learn item values naturally',        '2026-04-02 18:00:00'),
  -- More Spawn Points votes (request 3)
  (6,  3, 3, 0, 'Current spawns are fine, just learn the maps', '2026-04-03 11:00:00'),
  (7,  3, 4, 1, NULL,                                            '2026-04-03 14:00:00'),
  -- Expanded Task List votes (request 4, approved)
  (8,  4, 4, 1, 'Endgame is so dry right now',                  '2026-03-21 10:00:00'),
  (9,  4, 5, 1, NULL,                                            '2026-03-22 08:00:00'),
  (10, 4, 2, 1, 'Community quests look solid',                  '2026-03-23 14:00:00'),
  -- Full Auto Everything votes (request 6, rejected)
  (11, 6, 3, 0, 'This would ruin PvP balance',                  '2026-04-02 10:00:00'),
  (12, 6, 4, 0, 'Hard no from me',                              '2026-04-02 11:00:00'),
  (13, 6, 2, 0, 'Not appropriate for a competitive server',     '2026-04-03 09:00:00');

-- ============================================================
-- Pending operations (queued while server is running)
-- ============================================================
INSERT INTO pending_operations (id, action, forge_mod_id, forge_version_id, mod_name, metadata, queued_at, queued_by) VALUES
  (1, 'install', 850, 5501, 'Realism Mod', NULL, '2026-04-10 20:00:00', 'admin'),
  (2, 'remove',  1055, NULL, 'That''s Lit', NULL, '2026-04-10 20:05:00', 'admin');
