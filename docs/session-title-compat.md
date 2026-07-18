# Session Title Compatibility

`s7s`의 세션 제목 처리 로직은 외부 agent CLI의 내부 저장 구조에 강하게 의존한다.
이 저장 구조와 rename 동작은 agent 업그레이드 시 언제든 바뀔 수 있다.

이 문서는 다음을 기록한다.

- agent별 실제 제목 저장 위치
- `s7s`가 읽는 경로와 쓰는 경로
- 현재 확인된 rename 경로
- 변경 가능성이 높은 요소
- 구조 변경 시 확인 순서

## Volatile Warning

아래 요소는 모두 변경 가능하다.

- 세션 제목이 저장되는 파일 경로
- 세션 제목이 저장되는 필드 이름
- 세션 ID를 추출하는 방식
- 비대화형 CLI에서 rename이 가능한지 여부
- rename 성공 시 본문/메타 중 어디가 먼저 갱신되는지
- 세션 resume 명령의 옵션 이름과 동작

따라서 agent CLI가 업그레이드되면, 구현을 수정하기 전에 반드시 실제 로컬 환경에서 재검증해야 한다.

## Claude

### Read paths

- session body: `~/.claude/projects/<encoded-cwd>/<sessionId>.jsonl`
- session meta: `~/.claude/sessions/*.json`

### Title fields

- body events
  - `custom-title.customTitle`
  - `agent-name.agentName`
  - `ai-title.aiTitle`
- meta file
  - `sessionId`
  - `name`
  - `nameSource`

### Current rename strategy

1. `claude --resume <id> --name <title> -p --output-format json` 시도
   (세션 소속 프로필이 추가 경로면 `CLAUDE_CONFIG_DIR` 주입 + 오염 env 정리 —
   resume와 동일 규칙, 46차)
2. 실제 JSONL에 `custom-title` + `agent-name` 이벤트가 생겼는지 확인
3. 성공 시 그 결과를 신뢰
4. 실패 시 메타 JSON 갱신 + JSONL 이벤트 직접 append

### Verified behavior

- `--name`은 비대화형 환경에서도 제목 이벤트를 남긴다.
- `/rename ...` 프롬프트는 현재 print 환경에서 동작하지 않는다.
- 프롬프트 없이 `--name`만 호출하는 것은 deferred marker가 없으면 실패할 수 있다.
- 따라서 현재 구현은 `--name` 호출 후 실제 파일 변경 여부를 기준으로 성공을 판정한다.

### Failure modes

- CLI는 성공 exit code를 반환해도 제목 이벤트를 쓰지 않을 수 있다.
- JSONL 구조가 바뀌면 `custom-title`/`agent-name` 탐지가 깨질 수 있다.
- `~/.claude/sessions/*.json` 형식이 바뀌면 폴백 메타 갱신이 깨질 수 있다.

## Codex

### Read paths

- session body: `~/.codex/sessions/YYYY/MM/DD/rollout-<timestamp>-<sessionId>.jsonl`
- title index: `~/.codex/session_index.jsonl`
- title DB: `~/.codex/state_*.sqlite`의 `threads.title`

### Title fields

- body
  - `session_meta.payload.id`
- title index
  - `id`
  - `thread_name`
- sqlite
  - `threads.id`
  - `threads.title`

### Current rename strategy

- `session_index.jsonl`의 `thread_name` 갱신
- `state_*.sqlite`의 `threads.title` 갱신

### Verified behavior

- 비대화형 `codex exec resume <id> "/rename ..."`는 제목을 바꾸지 않았다.
- Codex는 `/rename` 프롬프트에 대해 제목 직접 변경이 불가능하다고 응답했다.
- 현재 시점에서는 외부 CLI 기반 rename 경로를 신뢰할 수 없다.

### Failure modes

- sqlite 스키마가 바뀌면 `threads.title` 갱신이 깨진다.
- `session_index.jsonl` 포맷이 바뀌면 명시적 제목 로딩이 깨진다.
- 세션 본문 파일명과 실제 `session_meta.payload.id`가 불일치할 수 있으므로 파일명 기반 매칭은 금지한다.

## Antigravity (agy)

### Read paths

- session DB: `~/.gemini/antigravity-cli/conversations/<conversationId>.db`
- title annotation: `~/.gemini/antigravity-cli/annotations/<conversationId>.pbtxt`
- metadata cache: `~/.gemini/antigravity-cli/cache/conversation_metadata.json`
- last conversation map: `~/.gemini/antigravity-cli/cache/last_conversations.json`

### Title fields

- annotation
  - `title:"..."`
- metadata
  - `conversations.<id>.summary.Title`
  - `conversations.<id>.summary.Preview`

### Current rename strategy

- `annotations/<id>.pbtxt`의 `title:"..."`
- `conversation_metadata.json`의 `summary.Title`

### Verified behavior

- 비대화형 `agy --print "/rename ..."`는 현재 메타데이터나 pbtxt에 rename 흔적을 남기지 않았다.
- `--conversation <id>`를 줘도 실제로는 현재 작업 디렉터리의 last conversation을 재사용하는 동작이 보였다.
- 현재 시점에서는 외부 CLI 기반 rename 경로를 신뢰할 수 없다.

### Failure modes

- `Preview`는 제목이 아니므로 `Title`과 동일 취급하면 안 된다.
- `pbtxt`가 없고 metadata만 갱신되는 버전이 나올 수 있다.
- 반대로 metadata가 비어 있고 pbtxt만 갱신되는 버전도 가능하다.

## `s7s` 구현 원칙

- rename이 쓰는 모든 메타 경로는 **세션 소속 프로필의 config 루트**(`Profile.path`)에서
  파생한다(`rename_session(&Profile, ...)`). 위 절들의 `~/.claude` 등 기본 경로 표기는
  builtin 프로필 기준 예시이며, 추가 프로필 세션은 해당 프로필 루트에 기록된다.
  프로필을 찾지 못하면 기본 경로로 폴백하지 않고 rename을 중단한다(오계정 기록 방지).
- 외부 CLI rename은 "실제 파일 변경"이 확인될 때만 성공으로 간주한다.
- 성공 판정 전에는 exit code를 신뢰하지 않는다.
- 외부 CLI rename이 검증되지 않은 agent는 저장소 직접 갱신 방식을 유지한다.
- 캐시 재사용 경로에서도 메타를 다시 입혀야 한다.
- 저장 구조가 바뀌면 캐시 버전을 올릴지 검토한다.

## 구조 변경 시 조사 순서

1. 대상 agent CLI의 `--help`와 resume 관련 서브커맨드를 다시 확인한다.
2. 임시 세션을 만들고 제목 변경을 실제로 시도한다.
3. 제목 변경 전후의 저장 파일 diff를 확인한다.
4. 세션 본문, 메타 파일, 보조 캐시, sqlite를 모두 조사한다.
5. 어떤 파일이 정본인지 확정한다.
6. 읽기/쓰기 경로를 문서와 코드에 동시에 반영한다.
7. 단위 테스트와 수동 검증 절차를 모두 갱신한다.

## 관련 코드

- [src/rename.rs](../src/rename.rs)
- [src/parser/claude.rs](../src/parser/claude.rs)
- [src/parser/codex.rs](../src/parser/codex.rs)
- [src/parser/antigravity.rs](../src/parser/antigravity.rs)
- [src/scan.rs](../src/scan.rs)
- [src/title.rs](../src/title.rs)

