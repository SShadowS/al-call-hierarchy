// beyond-1B.3b Task 3 fixture — interface return-type shape: `GetIFoo():
// Interface ICRFoo` must type the receiver as `ReceiverType::Interface`
// (polymorphic fan-out), never a concrete guess.
interface ICRFoo
{
    procedure Bar();
}
