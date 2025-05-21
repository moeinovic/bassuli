-- Rename Dicks table to Hemoroids
ALTER TABLE Dicks RENAME TO Hemoroids;

-- Rename trigger and function names
DROP TRIGGER IF EXISTS trg_check_and_update_dicks_timestamp ON Hemoroids;
ALTER FUNCTION check_and_update_dicks_timestamp() RENAME TO check_and_update_hemoroids_timestamp;

-- Recreate the trigger with the new name
CREATE OR REPLACE TRIGGER trg_check_and_update_hemoroids_timestamp BEFORE INSERT OR UPDATE ON Hemoroids
    FOR EACH ROW EXECUTE FUNCTION check_and_update_hemoroids_timestamp();

-- Update the function to have new error message
CREATE OR REPLACE FUNCTION check_and_update_hemoroids_timestamp()
    RETURNS TRIGGER
    LANGUAGE PLPGSQL
AS $$
BEGIN
    IF current_date = date(OLD.updated_at) THEN
        RAISE EXCEPTION 'You have already applied treatment to your hemorrhoid today!'
            USING ERRCODE = 'GD0E1';
    END IF;

    NEW.updated_at := current_timestamp;
    RETURN NEW;
END
$$;

-- Rename column length to protrusion_level 
ALTER TABLE Hemoroids RENAME COLUMN length TO protrusion_level;

-- Rename Dick_of_Day table to Hemoroid_of_Day
ALTER TABLE Dick_of_Day RENAME TO Hemoroid_of_Day;
ALTER TABLE Hemoroid_of_Day RENAME COLUMN winner_uid TO lowest_uid;
