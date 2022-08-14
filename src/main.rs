use futures::{
    channel::mpsc::{channel, Receiver},
    SinkExt, StreamExt,
};
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher, EventKind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap as Map;
use std::fs;
use std::path::Path;
use regex;

#[derive(Serialize, Deserialize, Debug, Clone)]
struct RunConfig {
    pub tasks: Map<String, TaskConf>,
    pub rules: Vec<Rule>,
    pub debounce_time: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Rule {
    pub test: String,
    pub watch_dir: String,
    pub emit: String,
    pub priority: Option<i32>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
enum TaskConf {
    Copy(CopyTaskConf),
    Move(MoveTaskConf),
    TryUnpack(TryUnpackTaskConf),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct CopyTaskConf {
    pub target: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct MoveTaskConf {
    pub target: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct TryUnpackTaskConf {
    pub pwd: String,
}

#[tokio::main]
async fn main() {
    let path = std::path::Path::new("config.yml");
    let file = String::from_utf8(fs::read(path).unwrap()).unwrap();
    let config: RunConfig = serde_yaml::from_str(&file).unwrap();
    let mut tasks = Vec::new();
    config.rules.iter().for_each(|rule| {
        regex::Regex::new(&rule.test).unwrap(); //  pre check
        tasks.push(async_watch_target(&rule.watch_dir, rule.clone(), config.clone()));
    });
    futures::future::join_all(tasks).await;
}

fn async_watcher() -> notify::Result<(RecommendedWatcher, Receiver<notify::Result<Event>>)> {
    let (mut tx, rx) = channel(1);

    // Automatically select the best implementation for your platform.
    // You can also access each implementation directly e.g. INotifyWatcher.
    let watcher = RecommendedWatcher::new(move |res| {
        futures::executor::block_on(async {
            tx.send(res).await.unwrap();
        })
    })?;

    Ok((watcher, rx))
}

async fn async_watch_target<P: AsRef<Path>>(path: P, rule: Rule, config: RunConfig) -> notify::Result<()> {
    let (mut watcher, mut rx) = async_watcher()?;

    // Add a path to be watched. All files and directories at that path and
    // below will be monitored for changes.
    watcher.watch(path.as_ref(), RecursiveMode::Recursive)?;

    while let Some(res) = rx.next().await {
        match res {
            Ok(event) => match event {
                Event {
                    kind: EventKind::Create(..),
                    paths,
                    ..
                } => {
                    let path = &paths[0];
                    let file_name = path.as_path().file_name().unwrap().to_str().unwrap();
                    let regex = regex::Regex::new(&rule.test).unwrap();
                    let matched = regex.is_match(file_name);
                    if !matched {
                        continue;
                    }
                    let task = config.tasks.get(&rule.emit).unwrap();
                    match task {
                        TaskConf::Copy(conf) => {
                            let target = Path::new(&conf.target).join(file_name);
                            fs::copy(path, target).unwrap();
                        }
                        TaskConf::Move(conf) => {
                            let target = Path::new(&conf.target).join(file_name);
                            fs::rename(path, target).unwrap();
                        }
                        TaskConf::TryUnpack(conf) => {
                            let pwd = Path::new(&conf.pwd);
                            let target = pwd.join(file_name);
                            if target.is_dir() {
                                continue;
                            }
                            let mut archive = zip::ZipArchive::new(fs::File::open(path).unwrap());
                        }
                    }
                }
                _ => println!("other {:?}", event)
            },
            Err(e) => println!("watch error: {:?}", e),
        }
    }

    Ok(())
}
