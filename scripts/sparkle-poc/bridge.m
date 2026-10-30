// Test-only driver: accepts the local A -> B update without UI automation.
#import <AppKit/AppKit.h>
#import <Sparkle/Sparkle.h>

static void record(NSString *event) {
    NSDictionary *info = NSBundle.mainBundle.infoDictionary;
    NSString *line = [NSString stringWithFormat:@"%@ version=%@ pid=%d\n",
        event, info[@"CFBundleVersion"], getpid()];
    FILE *file = fopen([info[@"PoCLogPath"] fileSystemRepresentation], "a");
    if (file) { fputs(line.UTF8String, file); fclose(file); }
}

void sparkle_poc_prepare(void) {
    NSDictionary *info = NSBundle.mainBundle.infoDictionary;
    // Fail closed: this auto-install driver must never run in the real app.
    if (![NSBundle.mainBundle.bundleIdentifier hasPrefix:@"com.krabs.codex-reset.sparkle-poc."] ||
        ![info[@"SUFeedURL"] hasPrefix:@"http://127.0.0.1:"]) abort();
    NSString *data = info[@"PoCDataPath"];
    setenv("CODEX_RESET_SETTINGS_DIR", data.fileSystemRepresentation, 1);
    setenv("CODEX_RESET_CACHE_PATH", [data stringByAppendingPathComponent:@"cache.json"].fileSystemRepresentation, 1);
    setenv("CODEX_RESET_API_BASE", "http://127.0.0.1:1", 1);
}

@interface PoCDriver : NSObject <SPUUserDriver>
@end
@implementation PoCDriver
- (void)showUpdatePermissionRequest:(SPUUpdatePermissionRequest *)request reply:(void (^)(SUUpdatePermissionResponse *))reply {
    reply([[SUUpdatePermissionResponse alloc] initWithAutomaticUpdateChecks:NO sendSystemProfile:NO]);
}
- (void)showUserInitiatedUpdateCheckWithCancellation:(void (^)(void))cancel { record(@"checking"); }
- (void)showUpdateFoundWithAppcastItem:(SUAppcastItem *)item state:(SPUUserUpdateState *)state reply:(void (^)(SPUUserUpdateChoice))reply {
    record([@"found " stringByAppendingString:item.versionString]);
    reply(item.informationOnlyUpdate ? SPUUserUpdateChoiceDismiss : SPUUserUpdateChoiceInstall);
}
- (void)showUpdateReleaseNotesWithDownloadData:(SPUDownloadData *)data {}
- (void)showUpdateReleaseNotesFailedToDownloadWithError:(NSError *)error { record(error.description); }
- (void)showUpdateNotFoundWithError:(NSError *)error acknowledgement:(void (^)(void))ack { record(@"no-update"); ack(); }
- (void)showUpdaterError:(NSError *)error acknowledgement:(void (^)(void))ack { record([@"ERROR " stringByAppendingString:error.description]); ack(); }
- (void)showDownloadInitiatedWithCancellation:(void (^)(void))cancel { record(@"downloading"); }
- (void)showDownloadDidReceiveExpectedContentLength:(uint64_t)length {}
- (void)showDownloadDidReceiveDataOfLength:(uint64_t)length {}
- (void)showDownloadDidStartExtractingUpdate { record(@"extracting"); }
- (void)showExtractionReceivedProgress:(double)progress {}
- (void)showReadyToInstallAndRelaunch:(void (^)(SPUUserUpdateChoice))reply { record(@"ready-to-relaunch"); reply(SPUUserUpdateChoiceInstall); }
- (void)showInstallingUpdateWithApplicationTerminated:(BOOL)terminated retryTerminatingApplication:(void (^)(void))retry { record(@"installing"); }
- (void)showUpdateInstalledAndRelaunched:(BOOL)relaunched acknowledgement:(void (^)(void))ack { record(@"installed"); ack(); }
- (void)dismissUpdateInstallation {}
@end

void sparkle_poc_start(void) {
    // Called only after the real Rust AppKit menu and background monitor start.
    dispatch_async(dispatch_get_main_queue(), ^{
        record(@"app-ready");
        if (![NSBundle.mainBundle.infoDictionary[@"CFBundleVersion"] isEqualToString:@"1"]) return;
        static SPUUpdater *updater;
        updater = [[SPUUpdater alloc] initWithHostBundle:NSBundle.mainBundle
            applicationBundle:NSBundle.mainBundle userDriver:[PoCDriver new] delegate:nil];
        NSError *error = nil;
        if (![updater startUpdater:&error]) { record([@"ERROR " stringByAppendingString:error.description]); return; }
        [updater checkForUpdates];
    });
}
