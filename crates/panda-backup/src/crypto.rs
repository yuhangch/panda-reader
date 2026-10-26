use age::secrecy::SecretString;
use anyhow::{Context as _, ensure};
use std::{
    fs::File,
    io::{BufReader, Write},
    path::Path,
};

/// Stream an archive through age passphrase encryption and atomically commit it.
pub fn encrypt_file(source: &Path, destination: &Path, passphrase: &str) -> anyhow::Result<()> {
    validate_passphrase(passphrase)?;
    let temp =
        tempfile::NamedTempFile::new_in(destination.parent().unwrap_or_else(|| Path::new(".")))?;
    let output = temp.as_file().try_clone()?;
    let encryptor = age::Encryptor::with_user_passphrase(SecretString::from(passphrase.to_owned()));
    let mut writer = encryptor.wrap_output(output)?;
    std::io::copy(&mut File::open(source)?, &mut writer)?;
    writer.finish()?;
    temp.as_file().sync_all()?;
    temp.persist(destination).map_err(|error| error.error)?;
    Ok(())
}

/// Decrypt an age archive to a private staging file and atomically commit it.
pub fn decrypt_file(source: &Path, destination: &Path, passphrase: &str) -> anyhow::Result<()> {
    validate_passphrase(passphrase)?;
    let temp =
        tempfile::NamedTempFile::new_in(destination.parent().unwrap_or_else(|| Path::new(".")))?;
    let decryptor =
        age::Decryptor::new(BufReader::new(File::open(source)?)).context("invalid age backup")?;
    let identity = age::scrypt::Identity::new(SecretString::from(passphrase.to_owned()));
    let mut reader = decryptor
        .decrypt(std::iter::once(&identity as &dyn age::Identity))
        .context("backup password is incorrect or archive is damaged")?;
    let output = temp.as_file().try_clone()?;
    let mut output = std::io::BufWriter::new(output);
    std::io::copy(&mut reader, &mut output).context("backup decryption failed")?;
    output.flush()?;
    output.get_ref().sync_all()?;
    temp.persist(destination).map_err(|error| error.error)?;
    Ok(())
}

fn validate_passphrase(passphrase: &str) -> anyhow::Result<()> {
    ensure!(
        !passphrase.trim().is_empty(),
        "backup password must not be empty"
    );
    ensure!(
        passphrase.chars().count() >= 8,
        "backup password must be at least 8 characters"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn age_file_round_trip_and_wrong_password_rejection() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("plain.zip");
        let encrypted = dir.path().join("backup.prbackup.age");
        let decrypted = dir.path().join("restored.zip");
        std::fs::write(&input, vec![0x5a; 32 * 1024]).unwrap();
        encrypt_file(&input, &encrypted, "a strong test passphrase").unwrap();
        assert!(decrypt_file(&encrypted, &decrypted, "a wrong test passphrase").is_err());
        decrypt_file(&encrypted, &decrypted, "a strong test passphrase").unwrap();
        assert_eq!(
            std::fs::read(input).unwrap(),
            std::fs::read(decrypted).unwrap()
        );
    }
}
