use base64::{Engine, engine::general_purpose::STANDARD};
use minisign_verify::{PublicKey, Signature};
use semver::Version;
use sha2::{Digest, Sha256};

pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Verify the artifact and its signed version before using any trusted-comment fields.
pub fn verify(
    bytes: &[u8],
    signature: &str,
    public_key: &str,
    version: &Version,
) -> Result<(), String> {
    let decode = |text: &str| -> Result<String, String> {
        String::from_utf8(
            STANDARD
                .decode(text.trim())
                .map_err(|_| "更新签名编码无效")?,
        )
        .map_err(|_| "更新签名内容无效".into())
    };
    let key = PublicKey::decode(&decode(public_key)?).map_err(|_| "更新公钥无效")?;
    let signature = Signature::decode(&decode(signature)?).map_err(|_| "更新签名无效")?;
    key.verify(bytes, &signature, true)
        .map_err(|_| "更新包签名校验失败，未安装任何文件")?;
    let signed = signature
        .trusted_comment()
        .split('\t')
        .find_map(|part| part.strip_prefix("version:"))
        .ok_or("更新签名未绑定版本，不能安装")?;
    let signed = Version::parse(signed.trim_start_matches('v')).map_err(|_| "签名版本无效")?;
    if &signed != version {
        return Err("更新包的签名版本与发布版本不一致，不能安装".into());
    }
    Ok(())
}
