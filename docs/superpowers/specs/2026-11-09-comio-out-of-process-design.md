# comio out-of-process 활성화 설계 (경로 A: 클래식 COM LocalServer)

작성 2026-11-09. 상태: 승인 대기.

## 의도

comio 서버는 지금 in-process 형태뿐이다. 호스트가 `winasync.dll`을 직접 로드하고,
reg-free 매니페스트나 HKCU `InprocServer32`로 활성화한다. COM/WinRT가 정의하는 서버
형태 중 **별도 프로세스에서 객체를 제공하는 형태**가 빠져 있다. 이번 작업의 목적은 특정
사용 사례가 아니라 **구현상 스펙의 완결**이다(사용자 결정).

완결의 기준은 다음과 같다. 같은 여섯 클래스를 별도 서버 프로세스에서 활성화할 수 있어야
한다. 그리고 모든 인터페이스가 프로세스 경계를 넘어서도 in-process와 같게 동작해야 한다.
비동기 연산, 이벤트 콜백, 스트림 읽기와 쓰기, 수명, 서버 장애가 모두 포함된다.

## 범위

### 목표

1. **서버 실행 파일** `winasync-server.exe`. 여섯 CLSID를 `CoRegisterClassObject`로
   제공하고, 마지막 참조가 사라지면 스스로 종료한다.
2. **프록시/스텁 DLL** `winasync_ps.dll`. `.winmd`에서 생성한 마샬링 코드만 담는다.
   대상은 우리 인터페이스 10개와 제네릭 인스턴스 3개다. 제네릭은
   `IAsyncOperation<ITransportStream>`, 그 completed handler,
   `TypedEventHandler<IConnectionListener, IConnectionReceivedArgs>`이다.
3. **메타데이터 보강**. 모든 타입에 `[Version(1)]`을 붙인다. midl은 버전이나 contract가
   없는 WinRT 타입을 거부한다(MIDL4072). out-of-process와 별개로 보더라도 메타데이터의
   결함이다.
4. **검증**. Rust 통합 테스트, C# 왕복, Python 왕복.

### 비목표

- **이름 기반 WinRT out-of-process (경로 B, MSIX `outOfProcessServer`)**. 별도 실험으로
  미룬다. 이유는 다음과 같다(2026-11-09 재측정).
  `HKLM\SOFTWARE\Microsoft\WindowsRuntime\ActivatableClassId`는 소유자가
  `TrustedInstaller`이고 `Administrators`에게는 읽기 권한만 있다. 따라서 관리자도 이
  키에 등록할 수 없다.
- 다음 형태들도 이번 범위에서 제외한다.
  - DllSurrogate(`dllhost.exe`) 방식
  - 서비스로 실행하거나 다른 계정으로 실행하는 방식
  - 원격 DCOM
  - 상승된 권한의 클라이언트
- 라이브러리가 레지스트리에 직접 쓰는 것. 자기등록을 없앤 결정(`activation.rs` 문서)은
  그대로 유지한다. 등록은 설치자의 몫이고, 이 레포에서는 테스트와 샘플 스크립트가 설치자
  역할을 한다.

### 안티골

- 프록시가 없어 일부 메서드만 동작하는 out-of-process. 인터페이스 하나라도 마샬링이 안
  되면 완결이 아니다.
- IID가 바뀌는 것. `[Version]`을 추가한 뒤 `bindings.rs`가 바이트 단위로 같다는 것을
  확인했다. 앞으로도 같아야 한다.
- 객체가 없는데도 서버 exe가 남아 있는 것. 활성화 직후 경합으로 종료하는 것.
- `cargo build/test/clippy --workspace`가 깨지는 것.

## 이미 확인한 사실

- **midl 처리.** `[Version(1)]`을 넣은 `.winmd`는 `winmdidl` → `midl /winrt`를 패치 없이
  통과한다. 13개 인터페이스 모두의 프록시 vtable이 생성됐고, 제네릭 3개도 포함된다.
  winmdidl이 `declare { }` 블록을 직접 만들어 준다.
  - 사용한 플래그: `/winrt /env x64 /ns_prefix /target NT62`,
    `/metadata_dir <UnionMetadata>`
  - include 경로: `winrt`, `shared`, `um`
- **시스템 인터페이스의 프록시.** 우리가 쓰는 시스템 인터페이스는 모두 HKCR에
  `ProxyStubClsid32`가 이미 있다. 따라서 직접 만들 필요가 없다. 확인한 목록:
  - `IAsyncAction`, `AsyncActionCompletedHandler`, `IClosable`
  - `IInputStream`, `IOutputStream`
  - `IAsyncOperationWithProgress<IBuffer,UInt32>`,
    `IAsyncOperationWithProgress<UInt32,UInt32>`
  - `IAsyncOperation<Boolean>`
- **Python 경로.** PyWinRT 4.1의 `winrt._winrt.wrap_interface(capsule, 타입)`은
  `"winrt.interface"`라는 이름의 PyCapsule을 받는다. 실행해서 확인하지는 않았다.

## 결정 사항

### 사용자가 결정한 것

- 경로 A(클래식 COM LocalServer, HKCU, 권한 불필요)를 이번에 완결한다. 경로 B는 나중에
  실험한다.

### 내가 결정한 것 (검토할 때 확인 바람)

**패키지 구성.** 워크스페이스 멤버를 둘 추가한다.

- `server/`: 패키지 `winasync-server`, bin. `dll/`처럼 얇게 유지한다. 로직은 라이브러리의
  `comio::server`에 둔다.
- `ps/`: 패키지 `winasync-ps`, cdylib `winasync_ps.dll`.
  - 내용물은 midl이 생성한 C 코드와 export shim(`DllGetClassObject`,
    `DllCanUnloadNow`)뿐이다. Rust 라이브러리에 의존하지 않는다.
  - 프록시를 `winasync.dll`에 넣지 않는 이유가 있다. 서버 exe도 스텁을 로드해야 하는데,
    그러면 서버 프로세스 안에 라이브러리 사본이 하나 더 올라간다. 정적 상태,
    `DllMain`, Winsock 참조가 모두 별도인 사본이다.
  - MIDL 프록시를 별도 DLL로 두는 것은 표준 구성이기도 하다.
  - 클라이언트와 서버가 둘 다 이 작은 DLL만 로드한다. `winasync.dll`은 바꾸지 않는다.

**프록시 C 파일의 생성과 컴파일.**

- `tools/winmdgen`이 `.winmd` 다음에 `winmdidl`과 `midl`을 실행한다. 결과 C 파일은
  `ps/generated/`에 체크인한다. `.winmd`와 `bindings.rs`를 체크인하는 것과 같은
  원칙이며, 일반 빌드에는 SDK midl이 필요 없다.
- `ps/build.rs`가 `cc` 크레이트로 컴파일한다.
  - `rpcrt4`를 링크한다.
  - `ENTRY_PREFIX=Prx`, `PROXY_CLSID_IS=<CLSID_PROXY_STUB>`를 정의한다.
  - export는 `.def`로 고정한다. `dll/`과 같은 방식이다.
- `CLSID_PROXY_STUB`은 새 GUID 하나로 정하고 고정한다.

**서버 수명.** 문서화된 패턴을 따른다.

- 서버는 MTA다.
- `REGCLS_MULTIPLEUSE | REGCLS_SUSPENDED`로 등록한 다음 `CoResumeClassObjects`를
  호출한다.
- 수명은 `CoAddRefServerProcess` / `CoReleaseServerProcess`로 센다. 카운트가 0이 되면
  COM이 클래스 객체를 자동으로 중단하므로, 활성화 직후의 경합이 생기지 않는다.
- **무엇을 세는지.** 기존 `ModuleRef`를 그대로 쓴다. 객체, 풀 작업, 비동기 op가 모두
  여기에 해당한다.
  - 진행 중 I/O를 가진 op도 수명에 포함된다. 첫 초안의 "I/O는 세지 않는다"는 틀렸다.
    풀 작업과 op 객체가 이미 `ModuleRef`를 들고 있기 때문이다.
  - 진행 중 I/O를 세는 것 자체도 옳다. 마무리 콜백이 돌기 전에 프로세스가 끝나면 안 된다.
- **예외는 클래스 팩토리다.** `ClassFactory`가 `ModuleRef`를 들고 있으면, 등록해 둔 팩토리
  여섯 개가 카운트를 영원히 0보다 크게 유지한다. 그러면 서버가 종료되지 않는다.
  - 서버 모드에서는 팩토리가 카운트되지 않는 참조를 갖게 한다.
  - in-process 모드(`DllCanUnloadNow`)에서는 지금처럼 센다. in-process에서는 호스트가
    들고 있는 팩토리가 실제로 모듈을 붙잡고 있기 때문이다.
- **`LockServer`.** 서버 모드에서는 `CoAddRefServerProcess` /
  `CoReleaseServerProcess`로 매핑한다.
- **피처 구성.** 라이브러리에 `server` 피처를 추가한다.
  - 이 피처는 `dylib`과 **공존할 수 있어야** 한다. `--workspace` 빌드에서는 피처가
    합쳐지기 때문이다. 그래서 상호 배제(`compile_error!`)는 쓰지 않는다.
  - 대신 서버 모드는 런타임 값으로 정한다. `comio::server::run()`이 시작할 때 프로세스
    플래그를 켜고, 이 플래그가 켜져 있을 때만 `ModuleRef`가 COM 서버 카운트를 호출한다.
  - 이 플래그 static은 `server` 피처가 있을 때만 존재한다. 전역 상태 게이팅 원칙과 같다.
- **종료 순서.** 카운트가 0이 되면 메인 스레드가 깨어나 다음 순서로 정리한다.
  1. `CoRevokeClassObject`
  2. `release_process_state`
  3. `CoUninitialize`
- **시작 후 유휴 타임아웃.** 시작한 뒤 60초 안에 활성화가 한 번도 오지 않으면 종료한다.
  서버가 실행된 직후 클라이언트가 죽은 경우를 위한 것이다.

**버퍼.**

- 서버는 바이트에 `IBufferByteAccess`로 접근한다. 이 인터페이스에는 프록시가 없다.
- 그래서 프로세스 경계를 넘는 버퍼는 **값으로 마샬링되는 버퍼**여야 한다. 즉 `IMarshal`로
  버퍼 마샬러(`RoGetBufferMarshaler`)를 쓰는 버퍼다.
  - 시스템 `Buffer`가 여기에 해당한다.
  - C#의 `AsBuffer()`도 해당할 것으로 보지만 확인하지 않았다. 이 부분은 C# 테스트로
    확인한다.
- 값으로 마샬링되지 않는 버퍼가 오면 `IBufferByteAccess` 캐스트가 실패한다. 이때 명확한
  HRESULT(`E_INVALIDARG`)와 오류 메시지를 반환한다.
  - 이 규칙은 WinRT의 관례와 같다. 사용자 정의 `IBuffer`가 프로세스 경계를 넘으려면 버퍼
    마샬러를 구현해야 한다.
  - 이 규칙은 문서에 적는다.
  - `IBuffer` 메서드만으로 복사하는 대체 경로는 만들지 않는다. `IBuffer` 메서드로는 바이트
    내용을 읽을 수 없기 때문이다.

**등록 형태.** 모두 HKCU 아래이고, 설치자가 쓴다.

- `Software\Classes\CLSID\{클래스}\LocalServer32` = exe 경로
- `Software\Classes\Interface\{IID}\ProxyStubClsid32` = `CLSID_PROXY_STUB`. 13개 IID
  모두에 쓴다.
- `Software\Classes\CLSID\{CLSID_PROXY_STUB}\InprocServer32` = `winasync_ps.dll` 경로,
  `ThreadingModel=Both`
- 라이브러리는 이 목록을 **데이터로 노출**하기만 한다.
  - 예: `activation::PROXIED_IIDS`, `CLSID_PROXY_STUB`
  - 레지스트리에는 쓰지 않는다.

**클라이언트 쪽 진입.**

- Rust: `CoCreateInstance(CLSCTX_LOCAL_SERVER)`
- C#: `CoCreateInstance`를 P/Invoke로 호출한 다음, `MarshalInspectable<object>.FromAbi`로
  감싸서 캐스트한다.
- Python: 다음 순서로 진행한다.
  1. `ctypes`로 `CoCreateInstance`를 호출한다.
  2. 얻은 포인터를 `"winrt.interface"` PyCapsule로 감싼다. 캡슐 소멸자가 Release를
     호출하므로 참조를 정확히 하나 넘겨야 한다.
  3. `wrap_interface`로 감싼다.

**테스트 (`tests/oop.rs`, comio 피처).**

- 테스트가 HKCU 키를 쓰고 Drop에서 지운다. `tests/activation.rs`의 `Registered` 패턴이다.
- 서버와 ps DLL은 `cargo build -p`로 중첩 빌드한다. `ensure_dll`과 같은 방식이다.
- 검증 항목:
  1. `CLSCTX_LOCAL_SERVER` 활성화 결과가 프록시다. `IClientSecurity` QI가 성공하는지로
     판단한다.
  2. 리스너 bind → 커넥터 connect → `ConnectionReceived` 콜백이 클라이언트로 온다 → 양방향
     읽기와 쓰기가 된다. 시스템 `Buffer`로 하고, `ReadAsync`의 **결과** 버퍼를 쓴다.
  3. 다음 호출들이 프로세스 경계를 넘어 동작한다.
     - `GetRuntimeClassName`
     - `ITransport::Transport`
     - `IClosable::Close`
     - 비동기 op의 `Cancel`
  4. 마지막 참조를 해제하면 서버 프로세스가 제한 시간 안에 종료한다.
  5. 서버를 강제 종료해도 클라이언트 호출이 크래시하지 않는다. 대신 RPC 오류
     (`RPC_E_DISCONNECTED` 또는 `RPC_S_SERVER_UNAVAILABLE`)를 반환한다.
  6. 값으로 마샬링되지 않는 사용자 정의 `IBuffer`를 넘기면 정해진 HRESULT로 실패한다.

**샘플.**

- `python/accept.py`와 `csharp/Program.cs`에 `--oop` 모드를 추가한다. 등록과 해제는
  샘플이 직접 한다.
- `stage.sh`는 서버와 ps 패키지도 빌드한다.

**문서.**

- `docs/comio-com-winrt-status.md`에 한국어로 기록한다.
- `activation.rs` 모듈 문서의 활성화 모델 절에 LocalServer를 추가한다.

## 작업 순서

1. RDL에 `[Version(1)]` 추가. 이미 완료했고 작업 트리에 있다. IID 불변 확인.
2. winmdgen에 winmdidl과 midl 단계를 추가하고, C 파일을 체크인한다.
3. `ps/` 패키지. 프록시 DLL을 빌드하고 export를 확인한다.
4. `server` 피처, `comio::server`, `ModuleRef`와 팩토리, `LockServer` 처리, `server/`
   패키지. 기존 in-process 테스트가 그대로 통과해야 한다.
5. `tests/oop.rs`.
6. C#과 Python의 `--oop` 모드.
7. 문서와 메모리 갱신.

## 빌더에게 맡기는 것

- midl 출력 파일 이름과 배치.
- 서버 PID를 테스트에서 확인하는 방법. 공개 ABI(`.rdl`)에는 넣지 않는다.
- 서버 종료 대기 시간과 테스트 타임아웃.

## 알려진 위험

- **클라이언트가 죽었을 때 rundown 지연.** COM은 핑 타임아웃(약 6분)이 지나야 서버 쪽
  참조를 회수한다. 그동안 서버가 남아 있는 것은 정상 동작이다. 문서에만 적는다.
- **STA 클라이언트.** 이벤트 콜백을 받으려면 메시지 펌프를 돌려야 한다. 테스트와 샘플은
  MTA로 돌린다.
- **상승된 권한의 클라이언트는 HKCU 등록을 보지 않는다.** 비목표다.
