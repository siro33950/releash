#import <XCTest/XCTest.h>
#include <errno.h>
#include <fcntl.h>
#include <sys/file.h>
#include <unistd.h>

static NSString *screenLockPath(void) {
  return [NSTemporaryDirectory()
      stringByAppendingPathComponent:[NSString stringWithFormat:@"releash-macos-e2e-%u.lock", getuid()]];
}

@interface ScreenIsolationTests : XCTestCase
@end

@implementation ScreenIsolationTests

+ (void)load {
  // E2E と全 worktree の統合テストで画面を共有する。終了時に OS がロックを解放する。
  int descriptor = open(screenLockPath().fileSystemRepresentation, O_CREAT | O_RDWR | O_CLOEXEC, 0600);
  if (descriptor < 0) {
    [NSException raise:NSInternalInconsistencyException format:@"Cannot open screen lock: %d", errno];
  }
  NSLog(@"Waiting for shared macOS test screen: %@", screenLockPath());
  while (flock(descriptor, LOCK_EX) != 0) {
    if (errno != EINTR) {
      int error = errno;
      close(descriptor);
      [NSException raise:NSInternalInconsistencyException format:@"Cannot lock test screen: %d", error];
    }
  }
  NSLog(@"Acquired shared macOS test screen");
}

- (void)test_画面入力_別プロセスが同時にロックを取得できない {
  // Given
  NSTask *contender = [[NSTask alloc] init];
  contender.executableURL = [NSURL fileURLWithPath:@"/usr/bin/env"];
  contender.arguments = @[
    @"python3", @"-c",
    @"import fcntl, sys\n"
     "with open(sys.argv[1], 'a') as lock:\n"
     "    try:\n"
     "        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)\n"
     "    except BlockingIOError:\n"
     "        sys.exit(0)\n"
     "    sys.exit(1)\n",
    screenLockPath()
  ];
  // When
  NSError *error = nil;
  if (![contender launchAndReturnError:&error]) {
    XCTFail(@"Cannot launch lock contender: %@", error);
    return;
  }
  [contender waitUntilExit];
  // Then
  XCTAssertEqual(contender.terminationStatus, 0);
}

@end
