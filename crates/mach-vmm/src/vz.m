#import <Virtualization/Virtualization.h>
#import <Foundation/Foundation.h>

@interface MachVMDelegate : NSObject <VZVirtualMachineDelegate>
@end

@implementation MachVMDelegate

- (void)guestDidStopVirtualMachine:(VZVirtualMachine *)virtualMachine {
    NSLog(@"[mach-vmm] Guest stopped the virtual machine.");
    CFRunLoopStop(CFRunLoopGetCurrent());
    CFRunLoopStop(CFRunLoopGetMain());
}

- (void)virtualMachine:(VZVirtualMachine *)virtualMachine didStopWithError:(NSError *)error {
    NSLog(@"[mach-vmm] Virtual machine stopped with error: %@", error);
    CFRunLoopStop(CFRunLoopGetCurrent());
    CFRunLoopStop(CFRunLoopGetMain());
}

@end

int vz_start_vm(
    const char *kernel_path,
    const char *initrd_path,
    const char *disk_path,
    const char *cidata_path,
    const char *shared_dir_path,
    const char *cmdline,
    uint32_t cpus,
    uint64_t memory_mb,
    const char *mac_addr,
    int in_fd,
    int out_fd
) {
    @autoreleasepool {
        if (![VZVirtualMachine isSupported]) {
            NSLog(@"[mach-vmm] VZVirtualMachine is not supported on this host.");
            return -2;
        }

        VZVirtualMachineConfiguration *config = [[VZVirtualMachineConfiguration alloc] init];
        [config setCPUCount:cpus];
        [config setMemorySize:memory_mb * 1024 * 1024];

        // Boot Loader: Direct Linux Kernel (Alpine) or UEFI (Ubuntu / Debian cloud images)
        if (kernel_path && strlen(kernel_path) > 0) {
            NSURL *kernelURL = [NSURL fileURLWithPath:[NSString stringWithUTF8String:kernel_path]];
            VZLinuxBootLoader *bootLoader = [[VZLinuxBootLoader alloc] initWithKernelURL:kernelURL];
            if (initrd_path && strlen(initrd_path) > 0) {
                NSURL *initrdURL = [NSURL fileURLWithPath:[NSString stringWithUTF8String:initrd_path]];
                [bootLoader setInitialRamdiskURL:initrdURL];
            }
            if (cmdline && strlen(cmdline) > 0) {
                [bootLoader setCommandLine:[NSString stringWithUTF8String:cmdline]];
            }
            [config setBootLoader:bootLoader];
        } else {
            if (@available(macOS 13.0, *)) {
                VZEFIBootLoader *efi = [[VZEFIBootLoader alloc] init];
                if (disk_path && strlen(disk_path) > 0) {
                    NSString *diskStr = [NSString stringWithUTF8String:disk_path];
                    NSString *machDir = [diskStr stringByDeletingLastPathComponent];
                    NSString *varPath = [machDir stringByAppendingPathComponent:@"efi_vars.bin"];
                    NSURL *varURL = [NSURL fileURLWithPath:varPath];
                    VZEFIVariableStore *varStore = nil;
                    if (![[NSFileManager defaultManager] fileExistsAtPath:varPath]) {
                        varStore = [[VZEFIVariableStore alloc] initCreatingVariableStoreAtURL:varURL options:0 error:nil];
                    } else {
                        varStore = [[VZEFIVariableStore alloc] initWithURL:varURL];
                    }
                    if (varStore) {
                        [efi setVariableStore:varStore];
                    }
                }
                [config setBootLoader:efi];
                NSLog(@"[mach-vmm] Initialized UEFI bootloader for cloud image");
            } else {
                NSLog(@"[mach-vmm] UEFI boot requires macOS 13 or newer.");
                return -3;
            }
        }

        // Serial console
        VZVirtioConsoleDeviceSerialPortConfiguration *serial = [[VZVirtioConsoleDeviceSerialPortConfiguration alloc] init];
        NSFileHandle *inHandle = [[NSFileHandle alloc] initWithFileDescriptor:in_fd closeOnDealloc:NO];
        NSFileHandle *outHandle = [[NSFileHandle alloc] initWithFileDescriptor:out_fd closeOnDealloc:NO];
        VZFileHandleSerialPortAttachment *attachment = [[VZFileHandleSerialPortAttachment alloc]
            initWithFileHandleForReading:inHandle
            fileHandleForWriting:outHandle];
        [serial setAttachment:attachment];
        [config setSerialPorts:@[serial]];

        // Storage Devices (Primary Root Disk + optional Cloud-Init Seed Disk)
        NSMutableArray *storageDevices = [NSMutableArray array];
        if (disk_path && strlen(disk_path) > 0) {
            NSURL *diskURL = [NSURL fileURLWithPath:[NSString stringWithUTF8String:disk_path]];
            NSError *diskErr = nil;
            VZDiskImageStorageDeviceAttachment *diskAttach = [[VZDiskImageStorageDeviceAttachment alloc]
                initWithURL:diskURL
                readOnly:NO
                error:&diskErr];
            if (diskAttach) {
                VZVirtioBlockDeviceConfiguration *blockDev = [[VZVirtioBlockDeviceConfiguration alloc] initWithAttachment:diskAttach];
                [storageDevices addObject:blockDev];
                NSLog(@"[mach-vmm] Attached primary block device: %s", disk_path);
            } else {
                NSLog(@"[mach-vmm] Failed to attach disk at %s: %@", disk_path, diskErr);
            }
        }

        if (cidata_path && strlen(cidata_path) > 0) {
            NSURL *ciURL = [NSURL fileURLWithPath:[NSString stringWithUTF8String:cidata_path]];
            NSError *ciErr = nil;
            VZDiskImageStorageDeviceAttachment *ciAttach = [[VZDiskImageStorageDeviceAttachment alloc]
                initWithURL:ciURL
                readOnly:YES
                error:&ciErr];
            if (ciAttach) {
                VZVirtioBlockDeviceConfiguration *ciBlock = [[VZVirtioBlockDeviceConfiguration alloc] initWithAttachment:ciAttach];
                [storageDevices addObject:ciBlock];
                NSLog(@"[mach-vmm] Attached cloud-init seed disk: %s", cidata_path);
            } else {
                NSLog(@"[mach-vmm] Failed to attach cloud-init disk: %@", ciErr);
            }
        }

        if ([storageDevices count] > 0) {
            [config setStorageDevices:storageDevices];
        }

        // Host Directory Sharing via VirtioFS & Apple Rosetta 2 for Linux
        NSMutableArray *dirSharing = [NSMutableArray array];

        if (shared_dir_path && strlen(shared_dir_path) > 0) {
            if (@available(macOS 12.0, *)) {
                NSURL *sharedURL = [NSURL fileURLWithPath:[NSString stringWithUTF8String:shared_dir_path]];
                VZSharedDirectory *sharedDir = [[VZSharedDirectory alloc] initWithURL:sharedURL readOnly:NO];
                VZSingleDirectoryShare *share = [[VZSingleDirectoryShare alloc] initWithDirectory:sharedDir];
                VZVirtioFileSystemDeviceConfiguration *fsConfig = [[VZVirtioFileSystemDeviceConfiguration alloc] initWithTag:@"mach_share"];
                [fsConfig setShare:share];
                [dirSharing addObject:fsConfig];

                // Legacy tag support for backward compatibility with machines created as norb_share
                VZVirtioFileSystemDeviceConfiguration *legacyFsConfig = [[VZVirtioFileSystemDeviceConfiguration alloc] initWithTag:@"norb_share"];
                [legacyFsConfig setShare:share];
                [dirSharing addObject:legacyFsConfig];

                NSLog(@"[mach-vmm] VirtioFS shared folder attached: %s (tags: mach_share, norb_share)", shared_dir_path);
            }
        }

#ifdef __arm64__
        if (@available(macOS 13.0, *)) {
            if ([VZLinuxRosettaDirectoryShare availability] == VZLinuxRosettaAvailabilityInstalled) {
                NSError *rosettaErr = nil;
                VZLinuxRosettaDirectoryShare *rosettaShare = [[VZLinuxRosettaDirectoryShare alloc] initWithError:&rosettaErr];
                if (rosettaShare) {
                    VZVirtioFileSystemDeviceConfiguration *rosettaDev = [[VZVirtioFileSystemDeviceConfiguration alloc] initWithTag:@"rosetta"];
                    [rosettaDev setShare:rosettaShare];
                    [dirSharing addObject:rosettaDev];
                    NSLog(@"[mach-vmm] Apple Silicon Rosetta 2 share attached (tag: rosetta)");
                } else {
                    NSLog(@"[mach-vmm] Rosetta share initialization failed: %@", rosettaErr);
                }
            } else {
                NSLog(@"[mach-vmm] Rosetta for Linux availability status: %ld", (long)[VZLinuxRosettaDirectoryShare availability]);
            }
        }
#endif

        if ([dirSharing count] > 0) {
            [config setDirectorySharingDevices:dirSharing];
        }

        // Memory Ballooning for dynamic RAM reclamation
        VZVirtioTraditionalMemoryBalloonDeviceConfiguration *balloon = [[VZVirtioTraditionalMemoryBalloonDeviceConfiguration alloc] init];
        [config setMemoryBalloonDevices:@[balloon]];

        // Virtual Network with NAT
        VZVirtioNetworkDeviceConfiguration *net = [[VZVirtioNetworkDeviceConfiguration alloc] init];
        if (mac_addr && strlen(mac_addr) > 0) {
            VZMACAddress *mac = [[VZMACAddress alloc] initWithString:[NSString stringWithUTF8String:mac_addr]];
            if (mac) {
                [net setMACAddress:mac];
            }
        }
        VZNATNetworkDeviceAttachment *nat = [[VZNATNetworkDeviceAttachment alloc] init];
        [net setAttachment:nat];
        [config setNetworkDevices:@[net]];

        // Hardware RNG (Virtio entropy)
        VZVirtioEntropyDeviceConfiguration *entropy = [[VZVirtioEntropyDeviceConfiguration alloc] init];
        [config setEntropyDevices:@[entropy]];

        NSError *error = nil;
        if (![config validateWithError:&error]) {
            NSLog(@"[mach-vmm] Configuration validation failed: %@", error);
            return -1;
        }

        VZVirtualMachine *vm = [[VZVirtualMachine alloc] initWithConfiguration:config];
        MachVMDelegate *delegate = [[MachVMDelegate alloc] init];
        [vm setDelegate:delegate];

        dispatch_semaphore_t sema = dispatch_semaphore_create(0);
        __block int start_result = 0;

        [vm startWithCompletionHandler:^(NSError * _Nullable err) {
            if (err) {
                NSLog(@"[mach-vmm] VM start error: %@", err);
                start_result = -1;
            } else {
                NSLog(@"[mach-vmm] Linux guest VM started successfully!");
            }
            dispatch_semaphore_signal(sema);
        }];

        // Pump the runloop until start completion callback fires
        while (dispatch_semaphore_wait(sema, dispatch_time(DISPATCH_TIME_NOW, 50 * NSEC_PER_MSEC)) != 0) {
            [[NSRunLoop currentRunLoop] runMode:NSDefaultRunLoopMode beforeDate:[NSDate dateWithTimeIntervalSinceNow:0.05]];
        }

        if (start_result != 0) {
            return -1;
        }

        // Run the main runloop continuously to process virtual device events
        CFRunLoopRun();
        (void)delegate;
        return 0;
    }
}
