use futures::{
    channel::mpsc::{channel, Receiver},
    SinkExt, StreamExt,
};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use regex;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::{collections::BTreeMap as Map, path::PathBuf};
use std::{fs, time::Duration};
use tokio::{task::JoinHandle, time::sleep};

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
    pub emit: Option<String>,
    pub priority: Option<i32>,
    pub execute: Option<TaskConf>,
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
        tasks.push(async_watch_target(
            &rule.watch_dir,
            rule.clone(),
            config.clone(),
        ));
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

async fn async_watch_target<P: AsRef<Path>>(
    path: P,
    rule: Rule,
    config: RunConfig,
) -> notify::Result<()> {
    let (mut watcher, mut rx) = async_watcher()?;

    // Add a path to be watched. All files and directories at that path and
    // below will be monitored for changes.
    watcher.watch(path.as_ref(), RecursiveMode::NonRecursive)?; //
    let mut map: Map<String, JoinHandle<()>> = Map::new();
    while let Some(res) = rx.next().await {
        match res {
            Ok(event) => match event {
                Event {
                    kind: EventKind::Create(..) | EventKind::Modify(..),
                    paths,
                    ..
                } => {
                    let path = &paths[0];
                    let path_str = path.to_str().unwrap().to_string();
                    let file_name = path
                        .as_path()
                        .file_name()
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .to_string();
                    let regex = regex::Regex::new(&rule.test).unwrap();
                    let matched = regex.is_match(&file_name);
                    if !matched {
                        continue;
                    }
                    if let Some(handler) = map.get(&path_str) {
                        handler.abort()
                    }
                    let sleep_time = Duration::from_secs(config.debounce_time.into());
                    let mut task = rule.execute.clone();
                    if task.is_none() {
                        if let Some(emit) = rule.emit.to_owned() {
                            task = Some(config.tasks.get(&emit).unwrap().clone());
                        }
                    }
                    let path = path.clone();
                    let hanler = tokio::spawn(async move {
                        println!("dispatching task: {:?}", task);
                        sleep(sleep_time).await;
                        println!("execute task: {:?}", task);
                        execute_task(task.unwrap(), file_name, path);
                    });
                    map.insert(path_str, hanler);
                }
                _ => println!("other {:?}", event),
            },
            Err(e) => println!("watch error: {:?}", e),
        }
    }

    Ok(())
}

fn execute_task(task: TaskConf, file_name: String, path: PathBuf) {
    match task {
        TaskConf::Copy(conf) => {
            let target = Path::new(&conf.target).join(file_name);
            println!("copy {} to {}", path.display(), target.display());
            fs::copy(path, target).unwrap();
        }
        TaskConf::Move(conf) => {
            let target = Path::new(&conf.target).join(file_name);
            // 移动文件到目标目录
            println!("move {:?} to {:?}", path.display(), target.display());
            match fs::rename(path.clone(), target.clone()) {
                Ok(_) => println!("move success"),
                Err(e) => {
                    println!("move failed: {:?}; use fallback method", e);
                    fs::copy(path.clone(), target).unwrap();
                    fs::remove_file(path).unwrap();
                    println!("move success")
                },
            }
        }
        TaskConf::TryUnpack(conf) => {
            let pwd = Path::new(&conf.pwd);
            let target = pwd.join(file_name);
            if target.is_dir() {
                return;
            }
            let mut _archive = zip::ZipArchive::new(fs::File::open(path).unwrap());
        }
    }
}
