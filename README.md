> **VibeDev AI 助手**，基于 [etkecc/baibot](https://github.com/etkecc/baibot)（[v2026.11.22.1](https://github.com/etkecc/baibot/releases/tag/v2026.11.22.1)）修改，AGPL-3.0。
>
> 本仓库 [VibeDev-Si/vibedev-ai-bot](https://github.com/VibeDev-Si/vibedev-ai-bot) 是「VibeDev 聊天」使用的公开 fork。
> 上游的 [LICENSE](./LICENSE)（AGPL-3.0）和版权声明保持不变；全部改动见文末的 [VibeDev changes](#vibedev-changes)。
> 以下到 [VibeDev changes](#vibedev-changes) 之前是上游 README 原文。

<p align="center">
	<img src="./etc/assets/baibot.svg" alt="baibot logo" width="150" />
	<h1 align="center">baibot</h1>
</p>

🤖 baibot is an [AI](https://en.wikipedia.org/wiki/Artificial_intelligence) ([Large Language Model](https://en.wikipedia.org/wiki/Large_language_model)) bot for [Matrix](https://matrix.org/) built by [etke.cc](https://etke.cc/) (managed Matrix servers).

The name is pronounced 'bye'-bot and is a play on [AI](https://en.wikipedia.org/wiki/Artificial_intelligence), referencing the fictional character [🇧🇬 Bai Ganyo](https://en.wikipedia.org/wiki/Bay_Ganyo).

It's designed as a more private and [featureful](#-features) alternative to [matrix-chatgpt-bot](https://github.com/matrixgpt/matrix-chatgpt-bot).
It's influenced by [chaz](https://github.com/arcuru/chaz), but does **not** use the [AIChat](https://github.com/sigoden/aichat) CLI tool and instead does everything in-process, without forking.


## 🌟 Features

- 🎨 Encourages **[provider](./docs/providers.md) choice** ([Anthropic](./docs/providers.md#anthropic), [Groq](./docs/providers.md#groq), [LocalAI](./docs/providers.md#localai), [OpenAI](./docs/providers.md#openai), [Venice](./docs/providers.md#venice) and [☁️ many more](./docs/providers.md#️-providers)) as well as **[mixing & matching models](./docs/features.md#-mixing--matching-models)**:

- Supports **different use purposes** (depending on the [☁️ provider](./docs/providers.md) & model):

  - [💬 text-generation](./docs/features.md#-text-generation): communicating with you via text (though certain models may "see" images as well). While chatting, the bot can use [🧰 Bot Tools](./docs/bot-tools.md), like reading the room's recent messages or creating images. The [OpenAI](./docs/providers.md#openai), [Anthropic](./docs/providers.md#anthropic) and [Venice](./docs/providers.md#venice) providers also support [🛠️ Provider Tools](./docs/features.md#️-provider-tools) (like web search)
  - [🦻 speech-to-text](./docs/features.md#-speech-to-text): turning your voice messages into text
  - [🗣️ text-to-speech](./docs/features.md#%EF%B8%8F-text-to-speech): turning bot or users text messages into voice messages
  - [🖌️ image-generation](./docs/features.md#image-generation): creating and editing images based on instructions

- 🪄 Supports [seamless voice interaction](./docs/features.md#seamless-voice-interaction) (turning user voice messages into text, answering in text, then turning that text back into voice)

- 🦻 Supports [transcribe-only mode](./docs/features.md#transcribe-only-mode) (turning user voice messages into text, without doing text-generation)

- 🗣️ Supports [text-to-speech-only mode](./docs/features.md#text-to-speech-only-mode) (turning user text messages into voice, without doing text-generation)

- 🔒 Supports [encryption](./docs/features.md#-encryption) for Matrix communication and Account-Data-stored configuration

- ♻️ Supports [context-management](./docs/configuration/text-generation.md#️-context-management) for every [provider](./docs/providers.md) (automatically trimming older messages on whole-turn boundaries once a conversation outgrows the context window)

- 🛠️ Allows **customizing much of the bot's [configuration](./docs/configuration/README.md)** at runtime (using commands sent via chat)

- 👥 **Actively maintained** by the team at [etke.cc](https://etke.cc/)


## 🖼️ Screenshots

![Introduction and general usage](./docs/screenshots/introduction-and-general-usage.webp)

You can find more screenshots on the [🌟 Features](./docs/features.md) and other [📚 Documentation](./docs/README.md) pages, as well as in the [docs/screenshots](./docs/screenshots) directory.


## 🚀 Getting Started

🗲 For a quick experiment, you can refer to the [🧑‍💻 development documentation](./docs/development.md) which contains information on how to build and run the bot (and its various dependency services) locally.

For a real installation, see the [🚀 Installation](./docs/installation.md) documentation which contains information on [🐋 Running in a container](./docs/installation.md#-running-in-a-container) and [🖥️️️️️ Running a binary](./docs/installation.md#%EF%B8%8F-running-a-binary).


## 📚 Documentation

See the bot's [📚 documentation](./docs/README.md) for more information on how to use and configure the bot.


## 💻 Development

See the bot's [🧑‍💻 development documentation](./docs/development.md) for more information on how to develop on the bot.


## 📜 Changes

This bot evolves over time, sometimes with backward-incompatible changes.

When updating the bot, refer to the [GitHub releases](https://github.com/etkecc/baibot/releases) to catch up with what's new (changes up to `v1.30.0` are in [the old changelog](https://github.com/etkecc/baibot/blob/v1.30.0/CHANGELOG.md)).


## 🆘 Support

- Matrix room: [#baibot:etke.cc](https://matrix.to/#/#baibot:etke.cc)

- GitHub issues: [etkecc/baibot/issues](https://github.com/etkecc/baibot/issues)

- (for [etke.cc](https://etke.cc/) customers): etke.cc [support](https://etke.cc/contacts/)


## VibeDev changes

基于上游标签 `v2026.11.22.1`，分支 `vibedev`。改动日期：2026-11-23。

产品约定：不做平台默认智能体；谁在群里添加 AI 助手，谁就在「VibeDev 集成」的设置页填自己的 API Key（费用由他的 Key 承担）；Key 只能通过设置页填写，不出现在聊天里。设置页的后端通过下面的内部接口配置机器人。

### 改动一览

改动集中在新模块 [`src/vibedev/`](./src/vibedev/)，上游文件里只有少量挂接点（都带 `VibeDev fork` 注释，方便以后合并上游）。

1. **内部配置 HTTP 接口**（[`src/vibedev/api/`](./src/vibedev/api/)）：读取、设置、删除某个群的 AI 助手，列出某个 API 的模型（接口见下文）。只在配置了监听地址时启用；共享密钥认证（常量时间比较）；全部 JSON；请求头 ≤ 16 KiB、请求体 ≤ 64 KiB；读请求头 10 秒、读请求体 5 秒、处理 29 秒（调用方最多等 30 秒）、访问模型服务 20 秒、连接 120 秒的上限；最多 64 个并发连接；Key 不出现在响应和日志里，错误里也不转出模型服务的响应内容。助手复用 baibot 自己的逻辑：固定 id 的 room-local 智能体 `vibedev-assistant`（provider `openai-compatible`），先用上游的 ping 试连，成功才保存，再设为本群的 text-generation 处理者，并写入本群访问规则。
2. **只访问公网**（[`src/vibedev/net.rs`](./src/vibedev/net.rs)）：`base_url` 只允许 https，不允许账号密码、查询串、片段、`localhost` 和不带点的主机名；拒绝回环、私有、链路本地、CGNAT、组播、文档/测试、保留地址和 IPv6 ULA 等非公网地址（IPv4 映射、NAT64、6to4 等内嵌 IPv4 的也算）。这些智能体的 HTTP 客户端用专门的 DNS 解析器：每次连接都重新解析并检查**全部**地址，只把检查过的地址交给连接器（防 DNS 重绑定）；不跟随重定向、不走代理、只允许 https。挂接点：openai-compatible 智能体配置新增字段 `public_network_only`（[`config.rs`](./src/agent/provider/openai_compat/config.rs)、[`controller.rs`](./src/agent/provider/openai_compat/controller.rs)）。
3. **日志脱敏**（[`src/vibedev/redact.rs`](./src/vibedev/redact.rs)）：接口处理过的 Key、以及所有 openai-compatible 智能体的 Key 都登记到脱敏表，日志输出前统一替换成 `[REDACTED]`（上游代码记录的服务商错误正文、调试输出也覆盖到）。挂接点：[`main.rs`](./src/main.rs) 的日志 writer。
4. **「AI 生成」标识**（[`src/vibedev/label.rs`](./src/vibedev/label.rs)）：AI 生成的文字回复默认在开头单独一段加 `【AI 生成】`（文字和位置可配置，不能去掉）；生成的图片、贴纸、语音的说明文字（caption）就是这个标识；语音转写和图片的改写提示也带标识。机器人自己的消息在作为上下文发给模型前会去掉标识，朗读（TTS）时也会去掉。挂接点：[`chat_completion/mod.rs`](./src/controller/chat_completion/mod.rs)、[`image/generation.rs`](./src/controller/image/generation.rs)、[`image/edit.rs`](./src/controller/image/edit.rs)、[`tools/image/mod.rs`](./src/tools/image/mod.rs)、[`utils/text_to_speech.rs`](./src/controller/utils/text_to_speech.rs)、[`conversation/matrix/utils/mod.rs`](./src/conversation/matrix/utils/mod.rs)。
5. **源代码链接**（AGPL-3.0 第 13 条）：机器人的帮助（`!bai help`）和加群介绍里给出本仓库链接，默认 `https://github.com/VibeDev-Si/vibedev-ai-bot`，可配置。挂接点：[`strings/introduction.rs`](./src/strings/introduction.rs)。
6. **配置**：配置文件新增 `vibedev` 一节和 `BAIBOT_VIBEDEV_*` 环境变量（[`src/vibedev/config.rs`](./src/vibedev/config.rs)）。挂接点：[`entity/cfg/config.rs`](./src/entity/cfg/config.rs)、[`bot/load_config.rs`](./src/bot/load_config.rs)、[`bot/implementation.rs`](./src/bot/implementation.rs)、[`lib.rs`](./src/lib.rs)。
7. **依赖**：`hyper`（http1 + server）、`hyper-util`、`http-body-util`、`subtle`，都已在上游依赖树中；`Cargo.lock` 只新增 `httpdate`（hyper 的 server 功能需要）。tokio 增加 `net` 功能。
8. **禁止在聊天里创建智能体：没有改代码。** 上游的 `agent create-room-local` 需要 `manage-agents` 权限，`create-global` 只有机器人管理员能用。部署配置里的全局访问规则只给本服用户 `invite`（邀请机器人进群），不给任何人 `use`、`manage-agents`、`manage-config`、`manage-access`；每个群谁能使用助手，由内部接口写入的本群规则决定（只授予 `text-generation`）。
9. 其他行为保持上游默认：群里要 @ 机器人或以命令前缀开头才回复（成员 ≤ 2 人的房间不需要）、话题串等；每个群的回复方式由接口的 `reply_style` 设置。
10. **机器人离开群时立即删除该群的助手配置**（[`src/vibedev/departure.rs`](./src/vibedev/departure.rs)，2026-11-23 补充）：机器人自己的成员状态变成 `leave` 或 `ban`（被踢、被封禁、群被关闭，或它自己退出）时，马上从该群配置里删除智能体 `vibedev-assistant`（连同加密保存的 Key）、指向它的 text-generation 处理者设置、访问规则 `vibedev-assistant` 和 `reply_style`，并写回 Matrix account data（Synapse 允许给已经不在的群写 account data）；别人设置的其他内容不动。写入失败会重试 3 次；机器人每次启动时还会把所有已离开/被封禁的群再检查一遍。不需要任何配置，没有改上游文件（处理器在 `vibedev::start` 里注册）。
11. **没人能在聊天里看到 Key**（2026-11-23 补充）：上游要求至少有一个机器人管理员（`access.admin_patterns` 为空时拒绝启动），所以管理员列表不能真的设成空；配置样例默认填一个不可能存在的账号 `@nobody:admin.invalid`，相当于没有管理员。同时在代码里：
    - `agent details-revealing-secrets` 对所有人（包括管理员）禁用，只回复一条说明（挂接点：[`controller/agent/details/mod.rs`](./src/controller/agent/details/mod.rs)）。`agent details` 照常可用，Key 显示为 `REDACTED`。
    - 智能体出错时发到群里的错误信息会把已登记的 Key 打码（服务商的错误信息可能带着 Key）（挂接点：[`strings/agent.rs`](./src/strings/agent.rs)）。
    - 影响：任何人都无法在聊天里查看任何智能体（包括全局、静态智能体）的 Key；上游建议的「先 `details-revealing-secrets` 再删除重建」来修改智能体配置的做法不再可行，重建时要自己重新填 Key；AI 助手的 Key 只能在设置页重新填写来更换。帮助文字里仍列出这个命令，但执行会被拒绝。

部署材料：配置样例 [`etc/vibedev/config.sample.yml`](./etc/vibedev/config.sample.yml)，密钥环境变量样例 [`etc/vibedev/secrets.env.example`](./etc/vibedev/secrets.env.example)（都只有占位符）。

### 内部接口

- 地址：`http://__BAIBOT_INTERNAL_HOST__:9101`（`vibedev.internal_api.listen_address`；只在容器内网可达，不要映射到宿主机或公网）。
- 认证：每个请求都要带 `Authorization: Bearer <密钥>`（`BAIBOT_VIBEDEV_INTERNAL_API_SECRET`，≥ 32 个字符）。缺少或不对一律 `401 VD_UNAUTHORIZED`（路径不存在也是 401）。
- 有请求体时必须是 `Content-Type: application/json`，≤ 64 KiB；未知字段会被拒绝（`400 VD_INVALID_REQUEST`），避免字段名写错时 Key 被悄悄当成「省略」。
- 路径里的 `{room_id}` 可以原样写，也可以百分号编码（推荐）；查询参数按表单编码解析（`encodeURIComponent` / `URLSearchParams` 都可以）。
- 响应都是 JSON（`Cache-Control: no-store`）。错误体只有两个字段：`{"errcode": "VD_…", "error": "<英文说明>"}`，不含 Key，也不含模型服务的响应内容。
- 「群管理员」由机器人根据自己同步到的群状态判断，不信任调用方：`acting_user` 是该群的**已加入成员**，且权力级别 ≥ 100（room v12 的创建者视为无限大）。
- 访问模型服务（试连、取模型列表）的超时是 20 秒；整个请求最多处理 29 秒。

#### 1. `GET /_vibedev/v1/rooms/{room_id}/assistant?acting_user=<mxid>`

一律返回 `200`（`acting_user` 缺少或格式不对时 `400`），**永远不返回 Key**。

群管理员看到完整信息：

```json
{
  "configured": true,
  "joined": true,
  "base_url": "https://api.vibedev.studio/v1",
  "model": "deepseek-chat",
  "system_prompt": "你是本群的助手。",
  "access": "everyone",
  "reply_style": "thread",
  "key_set": true,
  "key_set_by": "@alice:matrix.example.com",
  "updated_at": "2026-11-23T02:00:00Z"
}
```

- 没配置时 `configured: false`，`base_url`、`model`、`system_prompt`、`access`、`key_set_by`、`updated_at` 为 `null`，`key_set: false`；`reply_style` 是本群当前生效的回复方式（默认 `thread`）。
- `system_prompt` 为空串表示使用机器人默认的系统提示词。
- `key_set_by`：最后一次提供 Key 的用户；`updated_at`：最后一次保存的时间（RFC 3339，UTC）。

其他人只看到：

```json
{"configured": true, "joined": true}
```

机器人不在群里时，无论是谁都返回 `{"configured": false, "joined": false}`（读不到可信的群状态；而且机器人离开群时已经删除了该群的配置，见改动一览第 10 条）。

#### 2. `PUT /_vibedev/v1/rooms/{room_id}/assistant`

```json
{
  "acting_user": "@alice:matrix.example.com",
  "base_url": "https://api.vibedev.studio/v1",
  "api_key": "sk-...",
  "model": "deepseek-chat",
  "system_prompt": "你是本群的助手。",
  "access": "everyone",
  "reply_style": "thread"
}
```

| 字段 | 说明 |
| --- | --- |
| `acting_user` | 操作者的 Matrix ID，必填，必须是群管理员（否则 `403 VD_FORBIDDEN`） |
| `base_url` | 兼容 OpenAI 格式的接口地址，必填；只允许公网 https（规则见上文第 2 条），会做 DNS 检查，否则 `400 VD_BASE_URL_NOT_ALLOWED`。选「VibeDev」时填 `https://api.vibedev.studio/v1` |
| `api_key` | 可选。不传、`null` 或空串表示沿用已存的 Key——**只有已经存过 Key、并且 `base_url` 和已存的相同时才允许**，否则 `400 VD_API_KEY_REQUIRED`（防止同群另一个管理员把地址改成自己的服务器，让机器人带着别人的 Key 去试连）。地址按规范化后比较：协议和主机名大小写、默认端口 443、末尾的 `/` 不影响。Key 为 8–1024 个可见 ASCII 字符（不含空格、`"`、`\`），首尾空白会去掉 |
| `model` | 模型 id，必填，1–256 个字符 |
| `system_prompt` | 可选，≤ 16000 个字符；空串表示用默认提示词。支持 baibot 的变量，如 `{{ baibot_name }}` |
| `access` | `everyone`（所有成员可用）或 `admins`（仅权力级别 ≥ 100 可用） |
| `reply_style` | `thread`（在话题串里回复，上游默认）或 `reply`（直接回复消息） |

处理顺序：校验字段 → 机器人必须在群里（`409 VD_BOT_NOT_IN_ROOM`）→ 校验 `acting_user`（`403`）→ 解析并检查 `base_url` 的主机（`400`）→ 确定 Key（`400 VD_API_KEY_REQUIRED`）→ 用上游的 ping 试连（一次很短的对话请求，会用该 Key 产生极少量费用；20 秒超时；失败时按下表报错，什么都不保存）→ 保存智能体 `vibedev-assistant`、设为本群 text-generation 处理者、写入本群访问规则 `vibedev-assistant`（`everyone`：所有人可用 `text-generation`；`admins`：要求权力级别 ≥ 100）、设置本群 `reply_style`。带了新 Key 时 `key_set_by` 记为 `acting_user`，沿用 Key 时不变。

成功返回 `200`，内容与第 1 条的管理员视图相同。

#### 3. `DELETE /_vibedev/v1/rooms/{room_id}/assistant`

请求体 `{"acting_user": "@alice:matrix.example.com"}`，也接受 `?acting_user=`（两处都给时必须相同）。权限校验同 `PUT`，机器人不在群里时 `409`。删除智能体、处理者设置和本群规则（其他设置不动）。

成功返回 `200 {"configured": false}`；本来就没有配置也是这个结果，可以重复调用。

#### 4. `POST /_vibedev/v1/models`

返回 `{"models": ["deepseek-chat", "deepseek-reasoner"]}`：请求 `GET {base_url}/models`（OpenAI 格式的 `{"data": [{"id": …}]}`），去重、排序，最多 500 个；20 秒超时；响应最大 2 MiB。两种请求体：

- 用新填的 Key，不涉及具体的群：

  ```json
  {"base_url": "https://api.vibedev.studio/v1", "api_key": "sk-..."}
  ```

- 用这个群**已存的 Key 和已存的地址**（`acting_user` 必须是群管理员；机器人必须在群里）：

  ```json
  {"room_id": "!abc:matrix.example.com", "acting_user": "@alice:matrix.example.com"}
  ```

  如果同时带了 `base_url`，必须和已存的地址相同，否则 `400 VD_API_KEY_REQUIRED`；本群没有存 Key 也是 `400 VD_API_KEY_REQUIRED`。

两种混在一起（既有 `api_key` 又有 `room_id`）、`api_key` 没带 `base_url`、`room_id` 没带 `acting_user` 时返回 `400 VD_INVALID_REQUEST`；既没有 `api_key` 也没有 `room_id` 时返回 `400 VD_API_KEY_REQUIRED`。

#### 错误码

| HTTP | `errcode` | 含义 |
| --- | --- | --- |
| 400 | `VD_INVALID_REQUEST` | 请求格式不对（不是合法 JSON、缺字段、未知字段、取值不对）。路径不存在（404）、方法不对（405）、请求体太慢（408）、太大（413）、不是 JSON（415）也用这个 errcode，HTTP 状态码如括号所示 |
| 400 | `VD_BASE_URL_NOT_ALLOWED` | 地址不是 https、带账号密码/查询串/片段，或指向内网、解析不了 |
| 400 | `VD_API_KEY_REQUIRED` | 需要重新提供 Key（本群没有存过 Key，或 `base_url` 和已存的不同） |
| 401 | `VD_UNAUTHORIZED` | 缺少或错误的共享密钥 |
| 403 | `VD_FORBIDDEN` | `acting_user` 不是群管理员（不是已加入成员，或权力级别不足 100） |
| 409 | `VD_BOT_NOT_IN_ROOM` | 机器人不在群里 |
| 422 | `VD_INVALID_API_KEY` | 模型服务返回 401 或 403（或 4xx 且说明 Key 无效） |
| 422 | `VD_MODEL_NOT_FOUND` | 模型不存在（模型服务返回 4xx 且说明模型不存在） |
| 500 | `VD_INTERNAL` | 读写机器人配置或群状态失败 |
| 502 | `VD_UPSTREAM_ERROR` | 模型服务出错、连不上，或返回的不是 OpenAI 格式 |
| 504 | `VD_UPSTREAM_TIMEOUT` | 模型服务 20 秒内没有响应（或它自己返回 408/504/524） |
| 504 | `VD_TIMEOUT` | 整个请求 29 秒内没处理完（例如 Matrix 很慢） |

### 部署

1. **构建镜像**（用上游的 [Dockerfile](./Dockerfile)，需要 BuildKit，Docker 23+ 默认开启）：

   ```sh
   git clone https://github.com/VibeDev-Si/vibedev-ai-bot.git
   cd vibedev-ai-bot
   git checkout vibedev   # 或固定到某个提交
   docker build --build-arg BAIBOT_VERSION=2026.11.22.1-vibedev -t vibedev-ai-bot:2026.11.22.1-vibedev .
   ```

   `BAIBOT_VERSION` 只是写进 `baibot --version` 的版本号。

2. **准备账号**：在 Synapse 上注册普通用户 `vibedev-ai`（例如 `register_new_matrix_user`，不需要管理员）。`access.admin_patterns` 默认是不可能存在的账号（相当于没有机器人管理员），一般不用改。

3. **配置**：复制 [`etc/vibedev/config.sample.yml`](./etc/vibedev/config.sample.yml) 到服务器（例如 `/srv/vibedev-ai-bot/config.yml`），替换占位符；复制 [`etc/vibedev/secrets.env.example`](./etc/vibedev/secrets.env.example) 为 `secrets.env`（权限 600）并填写。`config_encryption_key` 必须设置并备份：各群的配置（包括 Key）都用它加密，丢了就要重新配置所有群。

4. **运行**（不加 `-p`，接口端口不映射出去；和集成中心后端放在同一个内部网络）：

   ```sh
   docker run -d --name vibedev-ai-bot --restart unless-stopped \
     --network __INTERNAL_NETWORK__ \
     -v /srv/vibedev-ai-bot/config.yml:/app/config.yml:ro \
     -v /srv/vibedev-ai-bot/data:/data \
     --env-file /srv/vibedev-ai-bot/secrets.env \
     vibedev-ai-bot:2026.11.22.1-vibedev
   ```

5. **集成中心**：接口地址填 `http://vibedev-ai-bot:9101`（容器名），密钥与 `BAIBOT_VIBEDEV_INTERNAL_API_SECRET` 相同；「添加扩展」时邀请 `@vibedev-ai:<server_name>`。设置页上也建议放本仓库的源代码链接。

注意：

- 访问规则：`initial_global_config` 只在机器人账号第一次启动时写入。账号已有全局配置时改 YAML 无效，需要机器人管理员在聊天里用 `!bai access` 调整：临时把运维账号填进 `admin_patterns`、重启，改完再换回不可能存在的账号。不要给任何人 `manage-agents` / `manage-config` / `manage-access`。
- 机器人管理员（如果填了真实账号）能改全局设置、访问规则、创建全局智能体，但看不到用户的 Key（`details-revealing-secrets` 已禁用，群里的错误信息会打码）。只填运维专用账号。
- 日志：不要对 `baibot` / `mxlink` 开 `trace` 级别。
- 机器人被移出群（被踢、被封禁、群被关闭）时，该群的助手配置和 Key 会立即删除；再被邀请回来需要在设置页重新配置。可以在测试站把机器人踢出群，日志里应出现 `Forgot the VibeDev assistant (and its API key) of a room the bot is no longer in`。

### 已知限制

- Key 加密后存放在机器人账号的 Matrix account data 里（`config_encryption_key`）；拿到机器人账号和该密钥的人可以解密（这是服务器层面的访问，聊天里看不到）。
- 离开群后的清理靠机器人同步到自己的离开事件；如果当时 homeserver 写入一直失败（重试 3 次），要等机器人下次启动时的检查才会删掉。Synapse 自身的数据库备份里可能还留有旧的（加密的）配置。
- 管理员列表因为上游的启动检查不能为空，只能填一个不可能存在的账号来达到同样效果。
- 内部接口本身没有 TLS 和限流，依赖内网隔离和共享密钥。
- 群管理员身份读自机器人同步到的群状态，可能比实际变化晚几秒。
- 「模型不存在」和「Key 无效」（非 401/403 时）是根据模型服务错误说明里的关键词判断的，判断不了时归为 `VD_UPSTREAM_ERROR`。
- VibeDev 智能体的请求不走代理（代理会自己解析域名，无法防内网访问），机器人所在环境需要能直连公网。
- 图片说明文字（caption）的显示取决于客户端是否支持 Matrix v1.10 的媒体说明；不支持时它作为图片名显示。
- 修改标识文字后，历史消息里的旧标识不会在上下文里被去掉。
- 日志脱敏只覆盖已登记的 Key（接口处理过的、本次启动后用过的 openai-compatible 智能体的）；重启后、某个群第一次用到助手之前，`trace` 级别的调试输出仍可能带 Key，所以不要开 `trace`。
- 只配置文字对话；语音转写、朗读、画图等功能没有为用户开放（产品决定），上游这些能力仍然只有机器人管理员能用。

### 开发

- 工具链按 [`rust-toolchain.toml`](./rust-toolchain.toml)（1.100.0）。检查：`cargo fmt --all -- --check`、`cargo clippy -- -D warnings`、`cargo test`。
- 合并上游：`git fetch upstream --tags && git merge <上游标签>`，冲突只会出现在带 `VibeDev fork` 注释的挂接点附近。
