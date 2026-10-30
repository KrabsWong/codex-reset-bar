// Main-thread-only bridge to Sparkle's standard UI and persisted preferences.
#import <AppKit/AppKit.h>
#import <Sparkle/Sparkle.h>
#include <stdbool.h>

static SPUStandardUpdaterController *controller;
static bool started;
static NSString *pendingVersion;
static void (*changeHandler)(const char *);

static void setPendingVersion(NSString *version) {
    pendingVersion = [version copy];
    if (changeHandler) changeHandler(pendingVersion.UTF8String);
}

@interface CodexUpdateDelegate : NSObject <SPUStandardUserDriverDelegate>
@end
@implementation CodexUpdateDelegate
- (BOOL)supportsGentleScheduledUpdateReminders { return YES; }
- (BOOL)standardUserDriverShouldHandleShowingScheduledUpdate:(SUAppcastItem *)update andInImmediateFocus:(BOOL)immediateFocus {
    // Even launch-time checks remain quiet until the user selects our reminder.
    return NO;
}
- (void)standardUserDriverWillHandleShowingUpdate:(BOOL)handleShowingUpdate forUpdate:(SUAppcastItem *)update state:(SPUUserUpdateState *)state {
    setPendingVersion(update.displayVersionString);
}
- (void)standardUserDriverWillFinishUpdateSession {
    // Dismissed/skipped updates and errors must not leave a stale reminder.
    setPendingVersion(nil);
}
@end
static CodexUpdateDelegate *delegate;

void codex_updater_set_handler(void (*handler)(const char *)) {
    changeHandler = handler;
    if (changeHandler) changeHandler(pendingVersion.UTF8String);
}
const char *codex_updater_version(void) { return pendingVersion.UTF8String; }

bool codex_updater_start(void) {
    if (controller) return started;
    delegate = [CodexUpdateDelegate new];
    controller = [[SPUStandardUpdaterController alloc] initWithStartingUpdater:NO
        updaterDelegate:nil userDriverDelegate:delegate];
    NSError *error = nil;
    started = [controller.updater startUpdater:&error];
    if (!started) NSLog(@"Could not start updater: %@", error);
    return started;
}

bool codex_updater_available(void) { return started; }
bool codex_updater_can_check(void) {
    // A deferred reminder is an active session: allow bringing it to the front.
    return started && (pendingVersion != nil || controller.updater.canCheckForUpdates);
}
void codex_updater_check(void) {
    if (codex_updater_can_check()) [controller checkForUpdates:nil];
}
bool codex_updater_automatic(void) { return started && controller.updater.automaticallyChecksForUpdates; }
void codex_updater_set_automatic(bool enabled) {
    if (started) controller.updater.automaticallyChecksForUpdates = enabled;
}
