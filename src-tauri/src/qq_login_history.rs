use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf, sync::Mutex};

const HISTORY_FILE_NAME: &str = "qq-login-history.json";
const MAX_REMEMBERED_ACCOUNTS: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservedQqAccount {
    pub qq_number: String,
    pub nickname: String,
    pub avatar_url: String,
}

pub struct QqLoginHistory {
    path: PathBuf,
    accounts: Mutex<Vec<ObservedQqAccount>>,
}

impl QqLoginHistory {
    pub fn new(data_dir: PathBuf) -> Self {
        let path = data_dir.join(HISTORY_FILE_NAME);
        let accounts = load_accounts(&path);
        Self {
            path,
            accounts: Mutex::new(accounts),
        }
    }

    /// 记录本次 login.enc 中观察到的账号，并返回“本次观察 ∪ 历史记录”。
    /// login.enc 在部分 QQ 版本中只保留最近一次登录，因此历史记录用于补齐其余仍在登录的账号；
    /// 历史中的账号仍需通过可见窗口昵称确认后才会被采用。
    pub fn remember_and_merge(&self, current: Vec<ObservedQqAccount>) -> Vec<ObservedQqAccount> {
        let mut accounts = self
            .accounts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let original = accounts.clone();
        for entry in current.iter().rev() {
            if let Some(index) = accounts
                .iter()
                .position(|account| account.qq_number == entry.qq_number)
            {
                accounts.remove(index);
            }
            accounts.insert(0, entry.clone());
        }
        accounts.truncate(MAX_REMEMBERED_ACCOUNTS);
        if *accounts != original {
            // 缓存写盘失败不能影响检测流程，内存中的记录仍然可用。
            let _ = persist_accounts(&self.path, &accounts);
        }

        let mut merged = current;
        for account in accounts.iter() {
            if !merged
                .iter()
                .any(|entry| entry.qq_number == account.qq_number)
            {
                merged.push(account.clone());
            }
        }
        merged
    }
}

fn load_accounts(path: &PathBuf) -> Vec<ObservedQqAccount> {
    let Ok(content) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(accounts) = serde_json::from_str::<Vec<ObservedQqAccount>>(&content) else {
        return Vec::new();
    };
    accounts.into_iter().take(MAX_REMEMBERED_ACCOUNTS).collect()
}

fn persist_accounts(path: &PathBuf, accounts: &[ObservedQqAccount]) -> Result<(), String> {
    let content = serde_json::to_vec_pretty(accounts)
        .map_err(|error| format!("序列化本机登录记录失败: {error}"))?;
    fs::write(path, content).map_err(|error| format!("保存本机登录记录失败: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(qq_number: &str, nickname: &str) -> ObservedQqAccount {
        ObservedQqAccount {
            qq_number: qq_number.to_owned(),
            nickname: nickname.to_owned(),
            avatar_url: format!("https://q1.qlogo.cn/g?b=qq&nk={qq_number}&s=100"),
        }
    }

    fn temp_history() -> (PathBuf, QqLoginHistory) {
        let directory = std::env::temp_dir().join(format!(
            "qq-farm-code-helper-history-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&directory).unwrap();
        let history = QqLoginHistory::new(directory.clone());
        (directory, history)
    }

    #[test]
    fn merges_previously_observed_accounts_into_the_current_login_list() {
        let (directory, history) = temp_history();
        history.remember_and_merge(vec![account("1343475483", "芜")]);

        let merged = history.remember_and_merge(vec![account("3170105001", "落")]);

        assert_eq!(
            merged
                .iter()
                .map(|entry| entry.qq_number.as_str())
                .collect::<Vec<_>>(),
            vec!["3170105001", "1343475483"]
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn remembers_accounts_across_store_instances() {
        let (directory, history) = temp_history();
        history.remember_and_merge(vec![account("1343475483", "芜")]);
        drop(history);

        let reopened = QqLoginHistory::new(directory.clone());
        let merged = reopened.remember_and_merge(vec![account("3170105001", "落")]);

        assert_eq!(merged.len(), 2);
        assert!(merged.iter().any(|entry| entry.qq_number == "1343475483"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn refreshes_the_remembered_nickname_when_it_changes() {
        let (directory, history) = temp_history();
        history.remember_and_merge(vec![account("1343475483", "旧昵称")]);

        history.remember_and_merge(vec![account("1343475483", "新昵称")]);
        let merged = history.remember_and_merge(vec![account("3170105001", "落")]);

        let remembered = merged
            .iter()
            .find(|entry| entry.qq_number == "1343475483")
            .unwrap();
        assert_eq!(remembered.nickname, "新昵称");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn keeps_only_the_most_recently_observed_accounts() {
        let (directory, history) = temp_history();
        for index in 0..(MAX_REMEMBERED_ACCOUNTS + 3) {
            history.remember_and_merge(vec![account(&format!("1000000{index:02}"), "账号")]);
        }

        let merged = history.remember_and_merge(Vec::new());

        assert_eq!(merged.len(), MAX_REMEMBERED_ACCOUNTS);
        assert!(!merged.iter().any(|entry| entry.qq_number == "100000000"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn ignores_a_corrupt_history_file() {
        let (directory, history) = temp_history();
        drop(history);
        fs::write(directory.join(HISTORY_FILE_NAME), b"not json").unwrap();

        let reopened = QqLoginHistory::new(directory.clone());
        let merged = reopened.remember_and_merge(vec![account("3170105001", "落")]);

        assert_eq!(merged.len(), 1);
        fs::remove_dir_all(directory).unwrap();
    }
}
