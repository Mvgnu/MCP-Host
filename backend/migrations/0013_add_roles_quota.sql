-- Add role and quota columns to users
ALTER TABLE users ADD COLUMN role TEXT NOT NULL DEFAULT 'user';
ALTER TABLE users ADD COLUMN server_quota INTEGER NOT NULL DEFAULT 5;
