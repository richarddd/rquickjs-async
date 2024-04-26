use std::{error::Error as StdError, time::Duration, usize};

use rquickjs::{
    async_with,
    context::EvalOptions,
    loader::{BuiltinResolver, FileResolver, ModuleLoader, ScriptLoader},
    prelude::Func,
    promise::Promise,
    AsyncContext, AsyncRuntime, CatchResultExt, Ctx, Function, IntoJs, Null, Object, Result,
    Undefined, Value,
};

fn print<'js>(ctx: Ctx<'js>, val: Value<'js>) -> Result<()> {
    let str = match ctx.json_stringify_replacer_space(val, Null, "  ".into_js(&ctx))? {
        Some(str) => str.to_string(),
        None => Ok("undefined".into()),
    }?;

    println!("{}", str);
    Ok(())
}

fn set_timeout_spawn<'js>(ctx: Ctx<'js>, callback: Function<'js>, millis: usize) -> Result<()> {
    ctx.spawn(async move {
        println!("before sleep");
        tokio::time::sleep(Duration::from_millis(millis as u64)).await;
        println!("after sleep");
        callback.call::<_, ()>(()).unwrap();
    });

    Ok(())
}

static SCRIPT: &str = r#"

async function main() {
  console.log("before promise");
  await new Promise((res) => setTimeout(res, 1000));
  console.log("after promise");

  setTimeout(() => {
    console.log("nested setTimeout 1");
    setTimeout(async () => {
      console.log("nested setTimeout 2");
      await new Promise((res) => setTimeout(res, 1000));
      console.log("nested setTimeout 3");

      blockUntilComplete(new Promise((res) => setTimeout(res, 1000)));

      console.log("blocking");
    }, 1000);
  }, 1000);
}

main().catch(console.log);


"#;

#[tokio::main]
async fn main() -> core::result::Result<(), Box<dyn StdError>> {
    let resolver = (
        BuiltinResolver::default(),
        FileResolver::default().with_path("."),
    );
    let loader = (ModuleLoader::default(), ScriptLoader::default());

    let rt = AsyncRuntime::new()?;
    rt.set_max_stack_size(512 * 1024).await;
    rt.set_gc_threshold(256 * 1024 * 1024).await;
    rt.set_loader(resolver, loader).await;

    let ctx: AsyncContext = AsyncContext::full(&rt).await?;

    async_with!(ctx => |ctx|{

        let res: Result<Promise> = (|| {
            let globals = ctx.globals();

            let console = Object::new(ctx.clone())?;
            console.set("log", Func::from(print))?;
            globals.set("console", console)?;

            globals.set(
                "blockUntilComplete",
                Func::from(move |ctx, promise| {
                    struct Args<'js>(Ctx<'js>, Promise<'js>);
                    let Args(ctx, promise) = Args(ctx, promise);

                    loop {
                        if let Some(x) = promise.result::<Value>() {
                            return x;
                        }

                        if !ctx.execute_pending_job() {
                            return Undefined.into_js(&ctx);
                        }
                    }
                }),
            )?;

            globals.set("setTimeout", Func::from(set_timeout_spawn))?;

            let mut options = EvalOptions::default();
            options.promise = true;
            options.strict = false;
            ctx.eval_with_options(SCRIPT, options)?
        })();

        match res.catch(&ctx){
            Ok(promise) => {
                if let Err(err) = promise.into_future::<Value>().await.catch(&ctx){
                    eprintln!("{}", err)
                }
            },
            Err(err) => {
                eprintln!("{}", err)
            },
        };

    })
    .await;

    rt.idle().await;

    Ok::<_, Box<dyn StdError>>(())
}
