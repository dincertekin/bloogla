-- A copy of each photo 400 pixels wide, for post cards and lists on phones
-- (the 800-pixel copy is in small_filename). Empty for images that don't
-- need one; older uploads get theirs when Bloogla starts.
ALTER TABLE media ADD COLUMN thumb_filename TEXT;
